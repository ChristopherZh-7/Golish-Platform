# Golish 平台架构优化设计

> 日期：2026-05-29
> 状态：Draft
> 来源：多 Agent 架构体检（后端 · 前端 · 主控综合）
> Relates to:
> - `docs/design/2026-05-25-extract-golish-asset-intel-crate.md`（crate 分层先例，保留其全部契约）
> - `docs/design/2026-05-20-agent-harness-strategy.md`（内层 domain harness，当前 deferred）
> - `AGENTS.md` §5 不变量 I1（错误码契约）/ I2（IDOR）/ I5（ts-rs 同步）
> 范围：backend Rust workspace（47 内部 crate）+ frontend React 应用
> **本文件只记录设计与路线图，不改任何代码、不动 `frontend/lib/generated/`。**

---

## 1. 背景与目标

### 1.1 背景

Golish 是 Tauri 2 桌面端的 agentic 渗透测试操作平台，后端为 50+ crate 的 Rust workspace，前端为 React 19 + TypeScript 6 + Vite 8 + Tailwind 4。随着 asset_intel / findings / pipeline / targets / organizations 等垂直域快速迭代，代码量已显著膨胀（`golish/src/tools/` 约 110 文件 / ~27k LOC），出现了**跨层重复**、**模块横向穿透**、**端到端契约（错误码 / ts-rs 类型）未落地**等结构性问题。

本设计基于一次只读架构体检（纯静态阅读，未改代码、未跑编译），把后端、前端、跨端三方结论固化为可执行的优化路线图，供后续按 `feature_list.json` 逐项推进。

### 1.2 目标

1. 用**带 `文件:行号` 证据**的问题清单替代「感觉上很乱」的模糊判断。
2. 定义 P0/P1/P2 三级优化路线图，每项含 **目标 / 影响面 / 验证 / 回滚**，可被独立挑选为一个 feature。
3. 把两条最高危的**端到端契约断裂**（I1 错误码、I5 ts-rs 同步）单列，作为后续 PR 的第一优先。

### 1.3 非目标

- 不在本文件内改任何代码 / 配置 / 生成物。
- 不重新设计业务语义（normalize 规则、pipeline 协议、provider fan-out 等保持不变）。
- 不启动内层 domain harness（见 §8，仍 deferred）。

---

## 2. 现状架构

### 2.1 后端（Rust workspace）

- **规模**：47 个内部 crate（含 4 个 `rig-*` fork），**无循环依赖**，可分 6 层。
- **分层**（证据：各 crate `Cargo.toml` 的 `[dependencies]`）：
  - **L0 基础**（零内部依赖）：`golish-platform` / `golish-settings` / `golish-udiff` / `golish-pentest-domain` / `golish-vuln-intel-domain` / `golish-json-repair` / `golish-js-analyzer` / `golish-graphiti` / `golish-projects`
  - **L1 核心**：`golish-core`（in-degree **22**，最高，被绝大多数 crate 依赖）
  - **L2 基础设施**：`golish-db` / `golish-shell-exec` / `golish-models` / `golish-events` / `golish-context` / `golish-session` / `golish-llm-providers` / `rig-*`
  - **L3 领域服务**：`golish-tools` / `golish-pentest` / `golish-vuln-intel` / `golish-pipeline` / `golish-scan-runner` / `golish-integrations` / `golish-sidecar`
  - **L4 Agent**：`golish-sub-agents` / `golish-agent-kit` / `golish-agent-runtime` / `golish-agent-bridge`
  - **L5 应用**：`golish`（out-degree **30**，god-crate，直接依赖几乎全部 vertical）
- **命令层**：`golish/src/tools/` 约 110 文件 / ~27k LOC / **308** 个 `#[tauri::command]`；全部命令在单个 `tauri::generate_handler!` 中注册，共 **533** 条（`backend/crates/golish/src/commands_registry.rs:36-210`）。
- **合规**（符合 AGENTS.md §2.2）：命令统一走 `commands_facade::*`，registry 无 `use crate::tools::*` glob、无 camelCase 命令名。

### 2.2 前端（React 应用）

- **栈**：React 19 + TypeScript 6 + Vite 8 + Tailwind 4；状态用 zustand 5（+immer）。
- **目录**：`frontend/components/<Feature>/`（约 65 个）；`frontend/lib/`（`api` / `target-panel` / `ai` / `events` / `util`）；store 切片在 `frontend/store/slices/*`（`frontend/store/index.ts:52`）。
- **状态管理混合**：全局走 zustand，但数据面板大量局部 `useState` + 轮询：
  - `frontend/lib/.../useTargetData.ts:26`（4 个事件监听 49-61 + 15s 轮询 62）
  - `frontend/components/TargetPanel/TargetGroupedView.tsx:125-159`（约 28 个 `useState`）
- **数据流**：组件 → `lib/api/<domain>` → `client.invoke`（`frontend/lib/api/client.ts:55`）→ Tauri；`client.ts:9-19,55-70` 已统一 `traceId` + `ApiError`。

---

## 3. 问题清单（逐条带 `文件:行号`）

### 3.1 后端 · 重复函数 / 逻辑

| # | 级别 | 问题 | 证据（`文件:行号`） |
|---|---|---|---|
| B-D1 | P0 | `golish-db` repo 层 CRUD 模板 10+ 类复制 | `get(id)`：`repo/findings.rs:38` / `repo/targets.rs:44` / `repo/vault.rs:46` / `repo/sessions.rs:24` / `repo/pipelines.rs:26`（13+ 文件同构）；`delete(id)`：`repo/findings.rs:82` / `repo/targets.rs:72` / `repo/methodology.rs:45`（16+ 文件）；`list(project_path)`：`repo/findings.rs:28` / `repo/targets.rs:34` / `repo/vault.rs:36`；JSON upsert：`repo/methodology.rs:7-23` 与 `repo/pipelines.rs:7-23` 除表名外逐字相同 |
| B-D2 | P0 | 作用域 SQL 在 repo 与 command 两层重写（IDOR 守卫分散） | `golish/src/tools/vault.rs`（5 处裸 SQL）/ `tools/findings/crud.rs`（7）/ `tools/methodology.rs`（3）/ `tools/pipeline/commands.rs`（2）；而 `golish-db` repo 已含等价 `IS NOT DISTINCT FROM` 逻辑 |
| B-D3 | P1 | 时间戳辅助函数 6+ 处重复 | `now_ts()`：`golish-pipeline/src/types.rs:95`、`golish/src/tools/wordlists.rs:51`、`tools/findings/mod.rs:142`、`tools/vault.rs:11`；`ts_from_dt()`：`tools/findings/mod.rs:149`、`tools/vault.rs:18`、`golish-vuln-intel/src/types.rs:7`；`now_ms()`：`golish/src/history/storage.rs:285`、`golish-core/src/api_request_stats.rs:75` |
| B-D4 | P1 | LLM provider 客户端构造 13 家双写 | `golish-agent-kit/src/llm_client/providers/*.rs` 的 `create_*_components` 复制 `golish-llm-providers/src/provider_trait/*.rs` 的 rig 构造；对照 `agent-kit/.../nvidia.rs:30-37` ≈ `llm-providers/.../nvidia.rs:42-50`（`api_key`/`base_url`/`build`/`completion_model`/`LlmClient::RigNvidia` 逐行一致） |
| B-D5 | P2 | `reqwest::Client` 各自构造 35+ 处，无共享超时/代理策略 | `golish-intel-providers/src/shared/http_common.rs` 已有雏形但未被复用 |

### 3.2 后端 · 强耦合

| # | 级别 | 问题 | 证据 |
|---|---|---|---|
| B-C1 | P0 | `GolishError` 序列化丢失 `code` 字段 | `golish/src/error.rs:133-139` 的 `Serialize` 仅输出 `to_string()` 纯字符串，无 `code` → 违反 AGENTS.md I1（错误带 `code`、前端按 map 翻译），前端只能字符串匹配 |
| B-C2 | P1 | `golish` god-crate；`tools` 子模块横向穿透 | `golish` out-degree 30；`tools/asset_intel/mod.rs:27` 直接 `use crate::tools::organizations::*`、`:30` `use crate::tools::pentest::PentestState`（同级模块互相依赖而非走抽象） |
| B-C3 | P2 | `golish-db` 无本地 `Error` enum | 全用 `anyhow::Result`，`thiserror` 是未用依赖（`golish-db/Cargo.toml:36`）；repo 错误到 `golish` 边界统一降级为 `Internal` 而非 `Database(sqlx::Error)` |
| B-C4 | P2 | `GolishError` 的 `#[from]` 覆盖不全 | `error.rs` 仅 `from` 了 Pty/Tool/Skills/Pentest/VulnIntel/Pipeline/ScanRunner；`IntelError` / `IntegrationError` / `GraphError` 经 `anyhow → Internal(String)` 降级丢类型 |
| B-C5 | P2 | `golish-graphiti` 隐性耦合 | crate 注释称共用 `golish-db` 的嵌入 PG，但 `golish-graphiti/Cargo.toml` 无 `golish-db` 依赖，直接 `sqlx`，运行时在 `golish` 拼装 |
| B-C6 | P2 | agent kit/runtime/bridge 依赖扇出重叠 | `golish-agent-bridge`（out 12）几乎重声明 `golish-agent-runtime`（out 11）的内部依赖 |

### 3.3 前端 · 重复

| # | 级别 | 问题 | 证据 |
|---|---|---|---|
| F-D1 | P1 | severity 色表多处重复 | 权威 `frontend/lib/severity.ts:13-35`，重复于 `components/FindingsPanel/FindingsPanel.tsx:36-50`（及 `497-507`）、`TargetDetail.tsx:96-100`、`lib/target-panel/asset-intel.ts:221`、`DagComponents.tsx:50-64`、`PipelinePanel.tsx:40-66` |
| F-D2 | P1 | 时间格式化未用足 `lib/time.ts` | `lib/time.ts:14-30` 未复用：`ProjectOverview/utils.tsx:18-25`、`AuditLogPanel/index.tsx:244-245`、`TargetSurfaceWorkbench.tsx:810-813` |
| F-D3 | P1 | 加载/空态未用共享组件 | 未用 `ui/AsyncView.tsx:38-45`：`FindingsPanel:457-460`、`PipelinePanel:305-309`、`ProjectOverview:180-181`、`DagComponents:689-693`；空态散写 `FindingsPanel:462-468`、`PipelinePanel:339-343` & `709-717` |
| F-D4 | P2 | 步骤图标 / candidate key / uuid 内联重复 | 步骤图标：`StepRow.tsx:45-60`、`PipelineProgressBlock.tsx:30-44` & `72-87`、`WorkflowProgress.tsx:36`；candidate key ×3：`TargetGroupedView.tsx:376,394` + `CandidateReviewList.tsx:106`；内联 `uuid()`：`PipelinePanel.tsx:33-35`、`useToolEditor.ts:17` |
| F-D5 | P2 | 两套 pipeline 进度 UI / 工具函数错位 / 裸事件监听 | `PipelineProgressBar.tsx:6-54` vs `PipelineProgressBlock.tsx`；`translateWithFallback` 错放 `org-fields.ts:213`；裸 `listen` 于 `useCaptureSession.ts:30` |

### 3.4 前端 · 强耦合 / 巨型组件

| # | 级别 | 问题 | 证据 |
|---|---|---|---|
| F-C1 | P1 | `TargetGroupedView` 巨型组件 | `TargetGroupedView.tsx`（743 行），约 28 个 `useState`；向 `OrgTreeSidebar` 传约 39 个 props（`658-698`）、向 `OrgWorkspacePanel` 传 24 props（`703-724`）；混 org CRUD + asset-intel（`292-370`）+ candidate（`372-420`） |
| F-C2 | P1 | 其它巨型面板 | `FindingsPanel.tsx`（656）、`DagComponents.tsx`（740）、`PipelinePanel.tsx`（724）、`CommandPalette`（27 props，`47-77`） |

---

## 4. 端到端契约断裂（最高优先）

AGENTS.md 自述「**类型不同步是端到端 bug 的头号来源**」（I5）。体检发现两条契约在前后端**两侧都未落地**，列为全局最高优先。

### 4.1 I1 · 错误码契约未落地

- **后端**：`GolishError::serialize`（`golish/src/error.rs:133-139`）只产出 `to_string()` 字符串，无稳定 `code`。
- **前端**：`FindingsPanel` / `PipelinePanel` / `ProjectOverview` 多条异步路径**没有 error 态**（吞错 → 白屏），违反 AGENTS.md §2.3「三态 UI」。
- **后果**：前端无法按 `code` 分支，只能脆弱地字符串匹配错误消息。

### 4.2 I5 · ts-rs 同步链形同虚设

- I5 要求跨 IPC 类型由 `#[derive(ts_rs::TS)]` 生成到 `frontend/lib/generated/`，但：
  - 该目录**不存在**；
  - 后端仅 1 个文件用了 ts-rs（`harness/types.rs`）；
  - 前端手写 `frontend/lib/ai/types.ts`（785 行），与后端存在**类型漂移**风险。
- 另有裸 `invoke` 绕过 api 层：`PipelinePanel.tsx:108` 直接 `invoke("pipeline_list")` 未走 `listPipelines`（`frontend/lib/api/pipeline.ts:11`）。

---

## 5. 优化路线图（P0/P1/P2）

> 每项含 **目标 / 影响面 / 验证 / 回滚**。一项 = 一个可挑入 `feature_list.json` 的 feature。

### P0 — 正确性 / 安全 / 端到端契约

#### P0-1 端到端错误码契约（I1）
- **目标**：`GolishError` 序列化为 `{ code, message }`；前端按 `code` map 翻译。
- **影响面**：`golish/src/error.rs`（序列化 + 各变体补 `code`）；前端 `lib/api/client.ts` 的 `ApiError` 解析；所有读 `error.message` 的面板。
- **验证**：`just test-rust`（error 序列化单测）+ `just test-fe`（ApiError 映射单测）+ 手动制造一个 `NotFound` 看前端是否按 code 渲染。
- **回滚**：序列化改动是叠加字段（仍含 message），前端旧字符串匹配可暂时并存；单 commit 可 revert。

#### P0-2 重建 ts-rs 同步链（I5）
- **目标**：跨 IPC 类型统一 `#[derive(ts_rs::TS)]`，生成回 `frontend/lib/generated/`；逐步替换手写 `lib/ai/types.ts`。
- **影响面**：后端各 DTO（findings/targets/pipeline/asset_intel 等）；前端 import 路径；`justfile` 生成步骤。
- **验证**：生成命令产出 `lib/generated/` 且 `just check-fe` 通过；diff 手写类型与生成类型确认无漂移。
- **回滚**：保留手写类型文件直到生成链稳定；生成目录可删、回退 import。

#### P0-3 作用域 SQL 下沉 `golish-db` repo（I2）
- **目标**：新增 `repo/scoped.rs`（`get_scoped` / `delete_scoped` / `list_by_project`，统一 `id=$1 AND project_path IS NOT DISTINCT FROM $2`）；命令层移除裸 SQL，统一调 repo + `ensure_scoped_mutation`。
- **影响面**：`golish-db/src/repo/`；`golish/src/tools/{vault,findings/crud,methodology,pipeline/commands}.rs`。
- **验证**：`just test-rust`（新增 IDOR 越权读写应返回 `NotFound` 的单测）；grep 确认 `tools/` 下不再有裸 `project_path = $` SQL。
- **回滚**：repo 新增为纯增量；命令层逐文件切换，未切换前旧路径仍工作。

#### P0-4 前端写操作回归 api 层 + 补 error 态
- **目标**：消除裸 `invoke`（`PipelinePanel.tsx:108` → `listPipelines`）；为 `FindingsPanel`/`PipelinePanel`/`ProjectOverview` 补 error 态。
- **影响面**：上述组件 + `lib/api/pipeline.ts`。
- **验证**：`just test-fe`；grep 组件层无直接 `invoke(`；手动断网/造错看 error UI。
- **回滚**：按组件粒度提交，逐个 revert。

### P1 — 可维护性

#### P1-1 `golish-db` generic scoped CRUD helper
- **目标**：用泛型 helper 收敛 B-D1 的 10+ 类重复，预计削减 `golish-db` repo 约数百~上千行。
- **影响面**：`golish-db/src/repo/*`。
- **验证**：`just test-rust` 全绿；行数对比。
- **回滚**：helper 为新增，repo 函数逐个迁移，未迁移的保持原状。

#### P1-2 agent-kit 复用 `LlmProvider` trait（消 B-D4）
- **目标**：`create_*_components` 调用 `golish-llm-providers` 的 `LlmProvider::create_client()` 拿 `LlmClient`，再附加 shared components，消除 13×~10 行双写。
- **影响面**：`golish-agent-kit/src/llm_client/providers/*.rs`。
- **验证**：`just test-rust`；对每个 provider 跑一次 client 构造冒烟。
- **回滚**：按 provider 逐个迁移。

#### P1-3 时间戳函数收敛 `golish-core`（消 B-D3）
- **目标**：统一 `now_ts` / `ts_from_dt` / `now_ms` 到 `golish-core` time util。
- **影响面**：`golish-core` + 6 处调用点。
- **验证**：`just test-rust`。
- **回滚**：保留旧函数为薄 re-export 一个版本周期。

#### P1-4 拆 `TargetGroupedView` 为 hooks/context（消 F-C1）
- **目标**：抽 `useOrgTree` / `useInlineOrgForms` / `useAssetIntelRuns` / `useOrgCandidates`，或引入 `TargetPanelContext` 取代 39-props 透传。
- **影响面**：`TargetPanel/` 一族组件。
- **验证**：`just test-fe` + `just test-e2e`（target 面板交互）。
- **回滚**：组件级提交，逐步替换。

#### P1-5 拆后端 monolith + 前端巨型面板
- **目标**：`organizations.rs`(766) / `methodology.rs`(579) / `vault.rs`(380) 仿 `findings/` 拆 `mod.rs`(types)+`crud.rs`(commands)[+`db.rs`]；前端拆 `FindingsPanel`（`useFindingsPanel`+`findingsConfig`+`FindingRow`）、`PipelinePanel`（`usePipelineEditor`/`PipelineSidebar`/`ToolPicker`）、`DagComponents`（`dagLayout`/`StepDetailPanel`/`StepNode`）。
- **影响面**：对应模块/组件，行为不变。
- **验证**：`just check` + 对应测试。
- **回滚**：纯结构性拆分，单 PR revert。

#### P1-6 共享前端展示组件 + severity/时间收敛
- **目标**：统一 `EmptyState`/`StatusBadge`/`StepStatusIcon`；severity → `StatusBadge`（消 F-D1）；时间格式统一 `lib/time.ts`（消 F-D2）；加载态统一 `AsyncView`（消 F-D3）。
- **影响面**：多面板。
- **验证**：`just test-fe`；视觉回归。
- **回滚**：组件级提交。

### P2 — 整洁 / 规范

#### P2-1 共享 `reqwest` client builder（消 B-D5）
- **目标**：统一超时/代理/UA 的 client 构造，复用 `http_common.rs`。
- **影响面**：35+ 调用点（渐进迁移）。
- **验证**：`just test-rust`。
- **回滚**：逐点迁移。

#### P2-2 补全 error `#[from]` + `golish-db` 本地 Error enum（消 B-C3/B-C4）
- **目标**：`GolishError` 补 `IntelError`/`IntegrationError`/`GraphError` 的 `#[from]`；`golish-db` 引入本地 `Error` enum（启用已声明的 `thiserror`）。
- **影响面**：`golish/src/error.rs`、`golish-db`。
- **验证**：`just test-rust`。
- **回滚**：叠加式改动，可 revert。

#### P2-3 命名规范 + workspace 纳入
- **目标**：`scan_*` → `scan_runner_*`；`vuln_intel` 的 `intel_*` 与 `intel_providers` 的 `intel_*` 命名冲突 → `vuln_intel_*` 前缀（I4）；`golish-pentest-mcp` 纳入 `backend/Cargo.toml` 的 `[workspace.members]`。
- **影响面**：命令名（需前端同步）、workspace 配置。
- **验证**：`just check`；前端 api 层同步更新。
- **回滚**：命名变更需前后端联动，建议单独 PR。

#### P2-4 `commands_facade/workspace.rs` 继续抽 facade
- **目标**：从 20-glob catch-all（`commands_facade/workspace.rs:44-63`）继续抽 `targets` / `methodology` / `security_analysis` 独立 facade。
- **影响面**：`commands_facade/`。
- **验证**：`just check`。
- **回滚**：纯重排，逐块迁移。

---

## 6. 风险与回滚

| 风险 | 说明 | 缓解 |
|---|---|---|
| 命名变更断前端 | P2-3 改命令名会断 `lib/api/*` 调用 | 命令名变更单独 PR，前后端同 commit，先加新名 alias 再删旧名 |
| ts-rs 迁移类型漂移 | P0-2 生成类型与手写不一致致编译/运行不符 | 先生成并 diff，保留手写文件到生成链稳定后再删 |
| 作用域 SQL 下沉漏改 | P0-3 漏迁某命令仍裸 SQL → IDOR 风险残留 | grep 兜底 + 越权单测，逐文件切换 |
| 大重构 scope 蔓延 | 拆分类任务易「顺手改无关代码」 | 每项一个 feature，遵守 AGENTS.md §3「不引入 scope 外改动」 |
| schema 兼容 | 若 P2-2 触及 migration | 遵守 I10：先扩字段、再上代码、再清旧字段 |

**统一回滚原则**：所有项均设计为「增量叠加 + 逐文件/逐组件迁移」，未迁移路径保持旧行为，单项可独立 revert。

---

## 7. 验证方式

| 层 | 命令 | 用途 |
|---|---|---|
| 全套门禁 | `just precommit`（= `just check` + `just test`） | commit 前必跑，全绿才提交（AGENTS.md §2.6） |
| 静态 + 单测 | `just check` | fmt + check-fe + test-fe + lint-rust + test-rust-all |
| 前端 | `just check-fe` / `just test-fe` | biome + typecheck / Vitest |
| 后端 | `just lint-rust` / `just test-rust` | clippy 零 warning / cargo nextest |
| E2E | `just test-e2e` | Playwright（target 面板、pipeline 交互回归） |

**完成定义**（对齐 AGENTS.md §3）：每项落地必须有实际跑过的验证命令 + 证据记录到 `agent-progress.md`，并把 `feature_list.json` 对应条目的 `verification` 逐条核对、填 `evidence`。**没有新鲜验证证据不许宣称完成。**

### 7.1 实施约定（工程效率）

> 来源：用户工程效率要求（2026-05-29）。

后端 Rust 重构落地时，采用 **「批量修改 → 统一编译 → 批量修错」** 节奏，避免「每改一处就编译一次」浪费时间：

1. **批量修改**：把一个任务（或一批相关改动）**全部改完**，期间**不要**逐处触发 `cargo build` / `cargo check`。
2. **统一编译**：全部改完后，**只**统一跑一次 `cargo build` / `cargo check`（或 `just check-rust`）。
3. **批量修错**：集中查看编译器报出的全部错误，**批量**修复，再统一编译验证；如此循环直至全绿。
4. **进入时机**：仅在「全部改完」后才进入编译-修错循环；中途不打断节奏做单点编译。

> 说明：本约定只影响**开发期编译节奏**，不改变本节最终验证门禁——合并前仍须 `just precommit` 全绿（fmt + lint + 前后端 test）。

---

## 8. 不在本次范围（deferred）

- **内层 domain harness**（stage gate / evidence ledger / Recon barrier）：见 `docs/design/2026-05-20-agent-harness-strategy.md`，当前 deferred，等信息收集闭环与工具 evidence 契约稳定后再启。
- **业务语义重设计**：normalize 规则、provider fan-out 协议、pipeline 执行语义不在本路线图内。
- **新增高风险扫描能力**（active scan / exploit）：需另起 `docs/design/` 写授权与 scope 边界（AGENTS.md §2.5）。
- **DB schema 大改 / 迁移**：除 P2-2 可能的 error enum 外，本路线图不主动改 schema；如需，走 I10 向后兼容流程。

---

> 本文档为只读架构体检的固化产物，所有结论均带 `文件:行号` 证据。后续每挑一项进入实现，请先在 `docs/superpowers/plans/` 写实现计划（`.cursor/skills/writing-plans/`），再按 `executing-plans` 推进。
