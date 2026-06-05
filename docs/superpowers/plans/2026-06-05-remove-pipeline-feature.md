# 移除 Pipeline 功能 实现计划

> **面向 AI 代理的工作者：** 必需子技能：`.cursor/skills/executing-plans` 逐 Phase 执行；改后用 `.cursor/skills/verification-before-completion` 收口。设计：`docs/design/2026-06-05-remove-pipeline-feature.md`。
>
> **目标：** 前后端彻底移除 pipeline 子系统（DAG 引擎 crate + 编辑器 UI + `pipeline_*` 命令 + `pipelines` 表 + `run_pipeline`/`flow_compose` AI 工具 + `pipeline_progress` 时间线可视化及其持久化配套）。
>
> **架构：** 这是一次**删除型重构**。按依赖反向拆 Phase：先迁移唯一的跨界依赖（`StoreStats`），再从「叶子引用」往「crate 本体」逐层摘除，最后删 crate / 删表 / 删前端 UI。每个 Phase 结束保持 `cargo check` 或 `tsc` 可编译并单独 commit。
>
> **技术栈：** Rust workspace（`backend/crates/`，nextest/clippy/fmt）+ React/TS（`frontend/`，biome/tsc/vitest）。统一命令走 `just`。

> **执行状态（2026-06-05 收口中）**：后端 Phase 0-4 由 MCP-2 完成（当时未 cargo 验证、未 commit）；前端 Phase 6-8 由 MCP-3 完成，`just check-fe` exit 0。**Phase 5 改为保留 `pipeline_only`**——执行期读码确认其为内部编排标志（reflector/refiner/orchestrator 的「不暴露为可委派 `sub_agent_*` 工具」开关），非 DAG pipeline，强删会致 orchestrator 自我委派回归。**Phase 8（D1）**：`ProjectOverview/` 经核实为孤儿死代码（无 importer + 后端驱动 `run_recon_pipeline` 已删），整目录删除。Phase 9（后端 cargo/clippy/nextest + precommit + commit）进行中。

---

## 前置约定

- **分支**：`git switch -c chore/remove-pipeline`（高风险，隔离）。
- **决策默认**（见设计 §3，执行前用户已确认）：D1=待定（默认删 PipelineProgressBar，备选重命名）、D2=移除 `pipeline_only`、D3=新增 drop migration。**Phase 8（D1）与 drop migration（Phase 4.x）执行前再次口头确认**。
- **验证命令**：
  - 后端单 crate：`cd backend && cargo check -p <crate>` / `cargo nextest run -p <crate> --status-level fail`
  - 后端全量：`cd backend && cargo nextest run --status-level fail`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`
  - 前端：`just check-fe`（biome + tsc）、`just test-fe`
  - 收口：`just precommit`
- **每 Phase 一个 commit**；message 见各 Phase 末。
- **铁律**：每次 StrReplace/Write 后 `ReadLints`；改前 grep 全量引用确认无遗漏。

---

## Phase 0 · 迁移 `StoreStats`（解除唯一跨界依赖）

**为什么**：`golish-pipeline/src/parser.rs` 的 `StoreStats` 被 `golish-pentest-app/src/output_parser.rs:296` re-export，供通用命令 `output_parse_and_store` 使用。不先迁移，删 crate 必编译失败。

**文件**：
- 修改 `backend/crates/golish-pentest-app/src/output_parser.rs`
- 排查 `StoreStats` 全量引用

**步骤**：
1. `grep -rn "StoreStats" backend/ frontend/`（用 Grep 工具）锁定所有使用点（预期：`golish-pipeline/parser.rs` 定义、`output_parser.rs` re-export、`output_parser.rs` 内 `ParseAndStoreResult.store_stats`、pipeline engine 内部使用、可能 `frontend/lib/pentest/api.ts` 类型）。
2. 在 `output_parser.rs` 用**本地定义**替换 re-export（字段须与原 `StoreStats` 完全一致，保持 serde 兼容）。原定义（`golish-pipeline/src/parser.rs`）字段以读取结果为准，逐字段照搬：

```rust
// output_parser.rs — 替换 `pub use golish_pipeline::parser::StoreStats;`
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct StoreStats {
    // ⚠ 执行时先 Read golish-pipeline/src/parser.rs 的 StoreStats，逐字段照抄，勿凭空写
    pub stored_count: usize,
    // ...其余字段照搬...
}
```

3. 删除 `output_parser.rs:293-296` 的注释 + `pub use golish_pipeline::parser::StoreStats;`。
4. 若有其它 crate 从 `golish_pipeline::...StoreStats` 引入（非 pipeline engine 内部），改为从 `golish_pentest_app::output_parser::StoreStats` 引入。

**验证**：`cd backend && cargo check -p golish-pentest-app`（此时 golish-pipeline 仍在，应绿）。
**commit**：`refactor(pentest-app): own StoreStats locally (decouple from golish-pipeline)`。

---

## Phase 1 · 后端摘除 AI/agent 侧 pipeline 引用（零行为变化）

> 这些工具早已 disabled，移除不改运行时行为。

**文件与编辑**：
1. `backend/crates/golish-pentest-app/src/pentest_bridge/mod.rs`
   - 删 `mod flow_compose;`(13) / `mod run_pipeline;`(18)、`pub use flow_compose::FlowComposeTool;`(22) / `pub use run_pipeline::RunPipelineTool;`(27)。
   - 删 `create_pentest_bridge_tools` 里关于 pipeline 的注释(36-40)、签名中 `_config_manager`/`_app_handle` 若仅 pipeline 用则评估是否保留（保留签名以免动调用方；改注释说明）。
   - 删 `AI_TOOL_CATALOG` 的 `run_pipeline`(120-124) / `flow_compose`(125-129) 两条 + 相关 doc(58-60,76-78,83-84)。
2. 删文件 `backend/crates/golish-pentest-app/src/pentest_bridge/run_pipeline.rs`、`flow_compose.rs`。
3. `backend/crates/golish-agent-runtime/src/execution_mode/policy.rs`：删 `run_pipeline`/`flow_compose` 字段(100-101) + 默认(127-128,144-145) + push 分支(167-171) + 测试断言(261-262)。
4. `backend/crates/golish-agent-runtime/src/execution_mode/prompt_render.rs`：删 `ToolRow` 两行(192-193)。
5. `backend/crates/golish-agent-runtime/src/execution_mode/modes/chat.rs`：删注释(7) + inline test 断言(81-82)。
6. `backend/crates/golish-agent-runtime/src/agentic_loop/tool_gate.rs`：删 `run_pipeline` gate 分支(42) + 测试 fixture(115)。
7. `backend/crates/golish-prompts/src/system_prompt/tests.rs`：(227-234) 断言「prompt 不含 run_pipeline」——可保留（仍成立）或删，按上下文取舍。

**验证**：`cd backend && cargo nextest run -p golish-pentest-app -p golish-agent-runtime --status-level fail` + `cargo check -p golish`。
**commit**：`refactor(agent): drop run_pipeline/flow_compose bridge tools + policy flags`。

---

## Phase 2 · 后端摘除 `pipeline_*` 命令 + facade + registry + `run_recon_pipeline`

**文件与编辑**：
1. 删文件 `backend/crates/golish/src/commands_facade/pipeline.rs`。
2. `backend/crates/golish/src/commands_facade/mod.rs`：删 `pub mod pipeline;`(44)。
3. `backend/crates/golish/src/commands_registry.rs`：
   - 删 `use commands_facade::pipeline::*;`(23)。
   - 删 handler 里 `run_recon_pipeline,`(44 行中)。
   - 删 `generate_handler!` 中 `pipeline_list … pipeline_list_ai_tools`(166-169)。
4. 删目录 `backend/crates/golish-pentest-app/src/pipeline/`（`mod.rs`/`commands.rs`/`storage.rs`）。
5. `backend/crates/golish-pentest-app/src/lib.rs`：删 `pub mod pipeline;`(13) + 相关模块 doc(4-8,35)。
6. `run_recon_pipeline` 移除：
   - `backend/crates/golish-agent-app/src/ai/commands/workflow.rs`：删 `run_recon_pipeline`(29-45)，**保留** `check_recon_tools_cmd`(49-93) 与 `ReconToolCheck`/`ToolStatus`。
   - `backend/crates/golish-agent-app/src/ai/mod.rs`：删 re-export(72)。
   - `backend/crates/golish/src/commands_facade/git_pty.rs`：删 doc 中 `run_recon_pipeline`(11)。
   - registry handler 已在 3 处理。

**验证**：`cd backend && cargo check -p golish`（应报「golish-pipeline 仍被 app-core/Cargo 依赖」但命令层应通过编译到 crate 依赖处；若 app-core error 变体仍在则 OK）。跑 `cargo nextest run -p golish-pentest-app -p golish-agent-app --status-level fail`。
**commit**：`refactor(commands): remove pipeline_* commands, facade, registry, run_recon_pipeline`。

---

## Phase 3 · 删除 `golish-pipeline` crate 本体 + 解除依赖

**文件与编辑**：
1. `backend/crates/golish-app-core/src/error.rs`：删 `Pipeline(#[from] golish_pipeline::PipelineError)`(56) + `Self::Pipeline(_) => "PIPELINE"`(97)。
2. `backend/crates/golish-events/src/domain_event.rs`：删 `Pipeline(PipelineEvent)` 变体(13,36-37) + `domain_name` 分支(51) + `PipelineEvent` enum(98-125)。
3. `backend/crates/golish-events/src/lib.rs`：删 `pub use ...PipelineEvent`(13)。
4. `backend/crates/golish-pentest-app/src/output_parser.rs`：确认已无 `golish_pipeline` 引用（Phase 0 已迁移）。
5. Cargo 依赖：
   - `backend/crates/golish/Cargo.toml`：删 `golish-pipeline`(58)。
   - `backend/crates/golish-app-core/Cargo.toml`：删 `golish-pipeline`(17)。
   - `backend/crates/golish-pentest-app/Cargo.toml`：删 `golish-pipeline`(20) + 描述里 pipelines 字样(6)。
   - `backend/Cargo.toml`：删 workspace member 两处(40,92) + `[workspace.dependencies]` 行(150)。
6. 删整个目录 `backend/crates/golish-pipeline/`。
7. `scripts/check_dag.py`：删 `golish-pipeline` 体积预算条目(93 附近)。
8. `backend/crates/golish-app-core/src/event_emitter.rs`(5)、`ports/pentest/tools.rs`(44)、`golish-pentest-app/src/pentest/tool_mgmt.rs`(68)、`golish/src/tools/mod.rs`(18,22,32)：清理提及 pipeline 的注释。

**验证**：`cd backend && cargo check --workspace` → 必须全绿（关键里程碑：crate 已不存在且无悬空依赖）。`cargo nextest run --status-level fail`。
**commit**：`refactor!: delete golish-pipeline crate + error/event/Cargo wiring`。

---

## Phase 4 · 后端删除 `pipelines` 表、repo、model（含 D3 drop migration）

**文件与编辑**：
1. 删文件 `backend/crates/golish-db/src/repo/pipelines.rs`。
2. `backend/crates/golish-db/src/repo/mod.rs`：删 `pub mod pipelines;`(22)。
3. `backend/crates/golish-db/src/models/pentest.rs`：删 `struct Pipeline`(182-187)。
4. `backend/crates/golish-db/src/repo/scoped.rs`：删 pipelines JSON 注释(183) + 测试 SQL(285-286,309-310)。
5. `backend/crates/golish/src/tools/project_io.rs`：删 pipelines 表导出(144-145) + 导入(301)。
6. `backend/crates/golish/src/projects/commands.rs`：删项目删除表清单里的 `"pipelines"`(152)。
7. **D3 drop migration**（执行前二次确认）：新建 `backend/crates/golish-db/migrations/20260605000001_drop_pipelines.sql`：

```sql
-- Forward-only cleanup: pipeline feature fully removed (see
-- docs/design/2026-06-05-remove-pipeline-feature.md). Drops user-saved pipelines.
DROP TABLE IF EXISTS pipelines;
```

8. `pentest_bridge/run_pipeline.rs` 内的 `SELECT data FROM pipelines` 已随 Phase 1 删文件移除——确认无残留 SQL 引用 `pipelines` 表（grep）。

**验证**：`cd backend && cargo nextest run -p golish-db -p golish --status-level fail`；本机起一次 embedded PG 跑迁移（或 `just test-rust` 覆盖迁移加载）确认 drop 迁移可用。
**commit**：`refactor(db)!: drop pipelines table + repo/model/project-io wiring`。

---

## Phase 5 · 后端清理 sub-agent `pipeline_only`（D2）

> pipeline 删除后该标志语义失效。移除字段，被标记 agent 转普通。

**文件与编辑**：
1. `backend/crates/golish-sub-agents/src/definition/mod.rs`：删 `pipeline_only` 字段(158) + builder `as_pipeline_only`(188,193-194)。
2. `backend/crates/golish-sub-agents/src/file_loader.rs`：删 YAML `pipeline_only` 读/写(50,171-172,270-271)。
3. `backend/crates/golish-sub-agents/src/executor/tool_setup.rs`：删 `pipeline_only` 跳过分支(57)。
4. `backend/crates/golish-sub-agents/src/defaults/builder/mod.rs`(157,207,270) + `registry.rs`(164,232,285)：删 `.as_pipeline_only()` 调用（3 个 agent 转普通；**执行时逐个确认**是否要彻底下线某 agent，默认转普通）。
5. `backend/crates/golish-agent-kit/src/tool_definitions/definitions.rs`：删 `.filter(|agent| !agent.pipeline_only)`(87) → 改为不过滤。
6. `backend/crates/golish-sub-agents/src/defaults/tests.rs`(107-109)：删/改 pipeline_only 相关断言。

**验证**：`cd backend && cargo nextest run -p golish-sub-agents -p golish-agent-kit --status-level fail`。
**commit**：`refactor(sub-agents): remove pipeline_only flag (pipeline feature gone)`。

---

## Phase 6 · 前端删除 DAG 编辑器 UI + API + 事件通道

**删文件**：
- `frontend/components/PipelinePanel/`（`PipelinePanel.tsx`, `DagComponents.tsx`）
- `frontend/components/TargetPanel/PipelineLauncher.tsx`、`pipelineValidation.ts`、`hooks/usePipelineForm.ts`
- `frontend/lib/api/pipeline.ts`、`frontend/lib/pentest/pipeline-types.ts`、`frontend/hooks/usePipelineEvents.ts`

**编辑**：
1. `frontend/App/lazyRegistry.ts`：删 `PipelinePanelView`(66-69)。
2. `frontend/App/AppShell.tsx`：删 import(35) + `{ view: "pipelines", ... }`(61)。
3. `frontend/App/hooks/useAppLifecycle.ts`：删 `usePipelineEvents` import/调用(10,37,69-70)。
4. `frontend/components/ActivityBar/ActivityBar.tsx`：删 `"pipelines"` 于 `ActivityView`/`BarItemId`(32,44) + 活动项(90) + `VIEW_ITEMS`(135)。
5. `frontend/lib/i18n/en.json:37` + `zh-CN.json:37`：删 `activity.pipelines` 键。
6. `frontend/lib/api/index.ts`：删 `import * as pipeline` + 导出(39,76,111)。
7. `frontend/lib/pentest/api.ts`：删 `pipeline_list_ai_tools` invoke(27-30)；`frontend/lib/pentest/types.ts`：删/改 `AiToolMeta` 的 Pipeline doc(48-55)。
8. `frontend/lib/tools.ts`：删 `run_pipeline: "Run Pipeline"`(118)。
9. `frontend/lib/api/error-codes.ts`：删 `PIPELINE`(40)（与后端 Phase 3 同步）。
10. 事件：`frontend/lib/events/channels.ts` 删 `PIPELINE_EVENT`(20)；`payloads.ts` 删 `PipelineStepInfo`/`PipelineStoreStats`/`PipelineEventPayload`(119-164)、map 项(192)、guard(217-228)；`index.ts` 删 re-export(11-13,21)；`listener.ts` 删 doc(40-42)。
11. `frontend/components/AIChatPanel/pentestSystemPrompt.ts`：重写——删 `run_pipeline` 主路径引导(67,96-99,135,181,184,193,195,227,229)，改述为直接调 recon 工具。
12. `frontend/lib/ai/session.ts`：删 `runReconPipeline`(126-137) + `triggerAutoRecon` 调用点(150-151)（连带评估 `triggerAutoRecon` 是否整体废弃）。
13. `frontend/components/AIChatPanel/ToolCallSummary.tsx`(50-55,383)、`frontend/services/ai-events/tool-handlers.ts`(101-107,172-174,262)、`frontend/components/TargetPanel/hooks/useTargetData.ts`(48,55,65)：删 `run_pipeline`/`pipeline-event` 处理分支。

**验证**：`just check-fe`（tsc 必绿）。
**commit**：`refactor(frontend): remove pipeline DAG UI, api, events, recon-pipeline caller`。

---

## Phase 7 · 前端删除 `pipeline_progress` 时间线可视化 + 持久化配套（S2，死代码）

**删文件**：
- `frontend/components/PipelineProgressBlock/`（`PipelineProgressBlock.tsx`, `index.ts`）
- `frontend/store/slices/workflow/pipeline.ts`、`frontend/store/types/pipeline.ts`

**编辑**：
1. `frontend/store/slices/workflow/index.ts`：删 `createPipelineActions` import/合并(7-8,15,125)。
2. `frontend/store/slices/workflow/plan.ts`：删 `syncPlanToPipeline`(160-213) + 顶部 pipeline 类型 import(1-8 中相关项)。
3. `frontend/store/slices/workflow/types.ts`：删 pipeline action 签名(5-6,17,23,97-132) + `subAgentPipelineMap`。
4. `frontend/store/slices/workflow/sub-agent.ts`：删把 sub-agent 挂到 `pipeline_progress` 块的逻辑(6,11,14,178-205,258+)，保留 sub-agent 其余功能。
5. `frontend/store/types/timeline.ts`：删 `pipeline_progress` 变体(2,11,33-35)。
6. `frontend/store/types/index.ts`(7,25-29)、`store/store-types.ts`(28-32)、`store/public-api.ts`(46-49)：删 pipeline 类型 re-export。
7. 持久化/恢复：`frontend/lib/workspace-storage.ts` 删 `PersistedPipelineBlock`(20,23-27,49)；`lib/conversation-db-sync.ts` 删 `pipeline_progress` 读写(262,495)；`lib/terminal-restore.ts` 删恢复/sanitize 分支(104,125,143-144,311)——保留对未知块类型的安全忽略；`lib/timeline/blockHeightEstimation.ts` 删高度估算(8,22,84-88)。
8. 锚点：`frontend/store/selectors/anchors.ts` 删 `P#`/`run_pipeline`/`pipeline_progress` 锚点(4,10,70,85-87,102-106)；`frontend/components/ui/AnchorChip.tsx` 删 `"pipeline"` AnchorKind + `P#` 样式(15,31,38-39)。
9. command 来源标记：`frontend/store/slices/session.ts`(65-66,106,167)、`session-terminal.ts`(70,182-183,191-192,278-280)、`session-draft-types.ts`(36) 删 `pipelineCommandSource`；`frontend/components/CommandBlock/CommandBlock.tsx`(22,64) 删 `"pipeline"` source badge。
10. mocks：`frontend/mocks/showcase.ts`、`frontend/mocks.ts` 删 pipeline mock helpers。

**验证**：`just check-fe` + `just test-fe`。
**commit**：`refactor(frontend): remove pipeline_progress timeline block + persistence wiring`。

---

## Phase 8 · 前端 recon 进度条（D1，执行前确认删 / 重命名）

> `ProjectOverview/PipelineProgressBar` 是 AI recon 进度 UI，名字带 pipeline 但非 DAG。

**若 D1=删除**：
- 删 `frontend/components/ProjectOverview/PipelineProgressBar.tsx`、`hooks/useReconFeed.ts`。
- `ProjectOverview.tsx`：删引用 + `pipelineActive`(8,29,61,104-105,145-146)。
- `ProjectOverview/types.ts`：删 `pipeline_start/done/error` 种类 + `PipelineProgress` 类型 + `RECON_STEPS`(35-37,61-68,111-112)；`utils.tsx`(44-48) 删 icon 映射。
- 其它仅展示用引用：`DashboardPanel/ActivityFeed.tsx`(11,22)、`AuditLogPanel/index.tsx`(50,77) 的 `pipeline_executed`/`"pipeline"` 分类按需清理。

**若 D1=重命名**（推荐）：
- `PipelineProgressBar` → `ReconProgressBar`，`useReconFeed` 保留，`types.ts` 的 `pipeline_*` feed 种类 → `recon_*`，更新 `ProjectOverview.tsx`/`utils.tsx` 引用；与 DAG 解耦，不丢功能。

**验证**：`just check-fe` + `just test-fe`。
**commit**：`refactor(frontend): {remove|rename} recon progress bar (decouple from pipeline)`。

---

## Phase 9 · 全量收口 + 文档/进度更新

**步骤**：
1. 收口验证：
```bash
just precommit   # = check（fmt + check-fe + test-fe + lint-rust + test-rust-all）+ test
```
2. 残留扫描：`grep -rin "pipeline" backend/crates frontend/components frontend/lib frontend/store`，确认只剩范围之外（设计 §6）的不相关词义。
3. `agent-progress.md`：新增本轮会话记录（目标 / 已删清单 / 跑过的验证 + 退出码 + 关键输出 / commit 列表 / 风险 / 下一步）。
4. `feature_list.json`：新增 `remove-pipeline-2026-06-05` 条目（`passing` + `evidence`）。
5. `docs/design/2026-06-05-remove-pipeline-feature.md` 头部 Status → Accepted/Implemented；本计划头加「执行状态」。

**commit**：`chore: record pipeline removal evidence + feature status`。

---

## 自检（writing-plans §自检）

**规格覆盖**（对照设计 §1-§3 子系统）：
- S1 引擎 crate → Phase 3；S1 命令/facade/registry → Phase 2；S1 AI 工具 → Phase 1；S1 DB 表 → Phase 4；S1 前端编辑器/API/事件 → Phase 6。
- S2 时间线可视化 + 持久化 → Phase 7。
- S3：`run_recon_pipeline` → Phase 2；`pipeline_only` → Phase 5（D2）；`PipelineProgressBar` → Phase 8（D1）。
- 跨界依赖 `StoreStats` → Phase 0；错误码契约 → Phase 3 + 6.9 两侧同步；drop 表 → Phase 4（D3）。

**编译可保持性（删除型关键）**：Phase 0 先解依赖 → Phase 1/2 摘叶子（crate 还在）→ Phase 3 删 crate（此时无悬空依赖）→ Phase 4-8 各自独立。每 Phase 末有 `cargo check`/`tsc` 关卡。

**无占位符**：每 Phase 列精确路径 + 行号 + 编辑动作；`StoreStats`/drop migration/error 变体给了实际代码；唯一「执行时再 Read 照抄」的是 `StoreStats` 字段（已显式标注须先 Read 原定义，避免凭空写字段——这是删除型计划里合理的「读真源」而非占位）。

**类型一致性**：`StoreStats` 迁移后字段名/serde 与原一致（Phase 0 强制照抄）；前后端错误码 `PIPELINE` 同步删（Phase 3 + 6.9）；`pipeline_progress` 块类型从 timeline union 删后，所有读写分支同 Phase 7 一并清。

**风险点复述**：drop migration 不可逆（D3 执行前二次确认）；`triggerAutoRecon`/`pentestSystemPrompt` 是行为相关改动（非纯删），Phase 6 须实测 AI recon 仍可走（改走直接工具调用）。

**YAGNI / 范围纪律**：不顺手重构无关代码；范围之外的 "pipeline" 词义（设计 §6）一律不动。
