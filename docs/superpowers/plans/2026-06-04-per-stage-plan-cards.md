# 两级 Per-Stage Plan 卡片 实现计划

> **面向 AI 工作者：** 用 superpowers:executing-plans 逐任务实现。每任务单独 commit。测试阶段无 flag，直接替换。
> 设计见 `docs/design/2026-06-04-per-stage-plan-cards.md`。

**目标：** task 模式每个 harness 阶段一张独立 plan 卡（当前阶段实时、历史阶段折叠），scoping 一进入就确定性出卡，不再"整条链塌成一张一直 update"。chat / 非 harness 单卡行为不变。

**技术栈：** Rust（golish-core / golish-agent-kit / golish-agent-runtime）+ React/TS（frontend）+ ts-rs。

---

## 任务 1 · `PlanUpdated` 加 `stage_id`（后端 IPC）

- **文件**：`golish-core/src/events/event.rs`
- **步骤**：`PlanUpdated` 增 `#[serde(default)] stage_id: Option<String>`（放在 `explanation` 后）。修所有构造点（grep `PlanUpdated {`：`tool_executors/plan.rs` ×2、`planner/manager/*`、`task_orchestrator/*`、characterization 测试等）先补 `stage_id: None` 占位编译过。
- **验证**：`cargo check -p golish-core -p golish-agent-kit -p golish-agent-runtime`；`just gen-types` 后 `git status` 看 `GeneratedAiEvent.ts` 变更。
- **提交**：`feat(events): add stage_id to PlanUpdated`

## 任务 2 · update_plan / patch 执行器透传 stage（后端）

- **文件**：`tool_executors/plan.rs`、`agentic_loop/tool_execution/direct/mod.rs`
- **步骤**：`execute_plan_tool` / `execute_plan_patch_tool` 增参 `stage_id: Option<&str>`，emit `PlanUpdated` 带上。调用点传 `ctx.harness_stage.map(|s| s.as_str())`。
- **验证**：`cargo nextest run -p golish-agent-kit -E 'test(plan)'`（既有 patch_tool_tests 调用点补参编译过 + 断言不回归）。
- **提交**：`feat(harness): tag PlanUpdated with active stage_id`

## 任务 3 · 阶段入口确定性种子卡（后端）

- **文件**：`task_orchestrator/subtask_phases/execute.rs`（`run_stage_subtasks`）
- **步骤**：进入 stage、跑 agent 前 `self.emit(AiEvent::PlanUpdated { stage_id: Some(stage.as_str()), version: 0, summary: {total:1,in_progress:1,..}, steps: [PlanStep{step: synthesize 的本阶段标题, status: in_progress, id:None, failure_kind:None}], explanation: None })`。
- **验证**：`cargo nextest run -p golish-agent-kit -E 'test(task_orchestrator)'`；新增断言 `stage_entry_emits_seed_plan`（捕获 emit 的事件含 stage_id + in_progress 种子）。
- **提交**：`feat(harness): emit deterministic per-stage seed plan card on stage entry`

## 任务 4 · 收紧 stage_execution_prompt（后端）

- **文件**：`task_orchestrator/prompts/mod.rs`
- **步骤**：`stage_execution_prompt` 增一句强约束：「只用 `update_plan` 列**本阶段** 2–5 条 todo，**不要**把其它阶段写进来；本阶段做完即 `submit_stage_deliverable`」。
- **验证**：`cargo nextest run -p golish-agent-kit -E 'test(prompts)'`（断言含关键词）。
- **提交**：`feat(prompts): constrain stage plan to current stage only`

## 任务 5 · 前端 store per-stage 路由

- **文件**：`store/types/session.ts`、`store/types/plan.ts`、`store/slices/workflow/plan.ts`、`services/ai-events/misc-handlers.ts`、`lib/generated/GeneratedAiEvent.ts`（任务 1 重生成）
- **步骤**：
  - `Session` 加 `plansByStage?: Record<string, TaskPlan>` + `stageOrder?: string[]`。
  - `handlePlanUpdated` 透传 `event.stage_id`。
  - `plan.ts` 加 `setStagePlan(sessionId, stageId, plan)`：写 `plansByStage[stageId]`（桶内 version 去重 + 种子 version 0 可被覆盖）、新 stage 追加 `stageOrder`。`setPlan` 旧路径保留给 `stage_id == null`。
- **验证**：`pnpm --silent typecheck`；新增 `plan.test`（路由进桶 + stageOrder 顺序 + 种子被覆盖）。
- **提交**：`feat(store): route plan updates into per-stage buckets`

## 任务 6 · 前端 StagePlanStack 渲染

- **文件**：新增 `components/AIChatPanel/StagePlanStack.tsx`；`hooks/useTaskPlanState.ts`（暴露 `plansByStage`/`stageOrder`）；`MessageBlock.tsx` / `AIChatPanel.tsx`（在 plan target 处：有 per-stage 数据则渲染 StagePlanStack，否则旧单卡）。
- **步骤**：StagePlanStack 按 `stageOrder` 映射，每 stage 一张 `InlinePlanCard`，头部 `prettyStageName(stageId)`（复用 `StageMarker` 的 `prettyStageName`）；当前含 in_progress 的卡默认展开，全 completed 的折叠。
- **验证**：`pnpm --silent typecheck` + `just test-fe`；新增 `StagePlanStack.test`（多 stage → 多卡 + 顺序 + 当前展开）。
- **提交**：`feat(ui): render per-stage plan cards (StagePlanStack)`

## 任务 7 · 收口

- **步骤**：`just gen-types` 确认无漂移；`just check-fe` + `just test-fe` + `cargo nextest`（受影响 crate）+ `cargo clippy`（受影响 crate）；`code-audit` 收口。
- **验证**：受影响范围全绿；手动 E2E：task『搞一下 example.com』→ scoping 一进就一张 scoping 卡，进 recon 再起一张，不再单张累积。
- **提交**：`chore(plan-ui): audit + verify per-stage plan cards`

---

## 自检

- **规格覆盖**：症状①（scoping 准时出卡）→ 任务 3 种子；症状②（多卡不累积）→ 任务 1/2/5/6 per-stage 桶。
- **类型一致**：`stage_id: Option<String>`(Rust) ↔ `stage_id?: string | null`(GeneratedAiEvent) ↔ handler 透传；`just check-types` 守门。
- **向后兼容**：`stage_id == null` 走旧单卡 + retiredPlans，chat 不受影响。
- **风险**：种子 version=0 与 PlanManager 全局 version 不冲突（桶内比较）；Part 2 的未来阶段灰显 roadmap 不在本次范围。
