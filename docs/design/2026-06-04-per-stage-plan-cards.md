# 两级 Per-Stage Plan 卡片（设计）

> 状态：proposed · 2026-06-04 · 关联根因诊断见同日会话；上游特性见 `2026-06-04-stage-internal-agent-todo-execution.md`

## 背景 / 问题

task 模式的 plan 卡当前是**会话级单例**：

- 后端一个 `PlanManager`，`update_plan` 整盘重写，emit `AiEvent::PlanUpdated { version, summary, steps, explanation }`（`golish-core/src/events/event.rs:379`）。
- 前端 `handlePlanUpdated → setPlan`（`store/slices/workflow/plan.ts`）整体替换会话 `plan`；仅当步骤"结构性变化"才把旧 plan 退休进 `retiredPlans`。`InlinePlanCard` 渲染单张卡。

新的"阶段内 agent-todo"执行让 depth-0 主 agent 在每个 harness 阶段用 `update_plan` 自管 todo。但因为 plan 是单例，模型把整条 kill-chain（scoping 子步 + recon/vuln/exploit/reporting）铺进**同一张清单**并原地累积更新 → 两个症状：

1. **scoping 期间无卡、事后才冒出来**：scoping 阶段若不调 `update_plan` 就没有 `PlanUpdated`，前端无卡；模型在下一步第一次 `update_plan` 时把 scoping 步骤**回溯式写成 completed** → 卡突然出现且前几步全绿。
2. **整条链塌成一张一直 update 的卡**，而非"每阶段一张"。

## 目标

- 每个 harness 阶段拥有**独立**的 plan 桶；前端按阶段渲染**多张卡**（当前阶段实时、历史阶段折叠）。
- scoping 一进入就出卡（确定性，不靠模型自觉）。
- 保留 chat 模式 / 非 harness 的单卡行为（向后兼容）。
- 不动 harness 骨架（stage DAG / gate / 证据 / 授权）。

## 关键事实（已核实）

- `AgenticLoopContext.harness_stage: Option<StageKind>`（`agentic_loop/context.rs:176`）在 `execute_plan_tool` 调用点（`agentic_loop/tool_execution/direct/mod.rs:57`）可见 → update_plan 可知当前 stage。
- 编排器（`run_stage_subtasks`，`task_orchestrator/subtask_phases/execute.rs`）能 `self.emit(AiEvent::...)`（已 emit TaskProgress / SubtaskCompleted）→ 可在阶段入口确定性 emit 一条 stage-tagged 的 PlanUpdated 种子。
- 前端 `TaskPlan / PlanStep / PlanSummary` 已不再 ts-rs 生成（`store/types/plan.ts` 内联）；但 `AiEvent`（含 PlanUpdated）仍 ts-rs 生成（`lib/generated/GeneratedAiEvent.ts`）→ 改 PlanUpdated 需 `just gen-types` 重生成（不变量 I5）。

## 设计

### 后端

1. **`PlanUpdated` 加 `stage_id: Option<String>`**（event.rs）。`#[serde(default)]` 向后兼容。
2. **update_plan / patch 执行器透传 stage**：`execute_plan_tool` / `execute_plan_patch_tool` 增参 `stage_id: Option<&str>`，调用点传 `ctx.harness_stage.map(|s| s.as_str())`；emit 时带上。
3. **阶段入口确定性种子**：`run_stage_subtasks` 进入某 stage、运行 agent 之前，`self.emit(AiEvent::PlanUpdated { stage_id: Some(stage), version: 0, steps: [<该阶段 charter 单条 todo, in_progress>], summary, explanation: None })`。保证每个阶段（含 confirm-only scoping）一进去就出卡；模型随后的 tagged update_plan 覆盖细化。
4. **`stage_execution_prompt` 收紧**："只列**本阶段** 2–5 条 todo，勿把其它阶段列进来。"

### 前端

5. **Store**：`Session` 增 `plansByStage?: Record<string, TaskPlan>` + `stageOrder?: string[]`（按首次出现顺序）。`handlePlanUpdated`：`stage_id` 有值 → 路由进 `plansByStage[stage_id]`（新 stage 追加 stageOrder）；为空 → 旧 `plan` 单卡路径（chat 向后兼容）。桶内按 `version` 去重。
6. **渲染**：新增 `StagePlanStack`：按 `stageOrder` 渲染，每 stage 一张 `InlinePlanCard`（头部 `prettyStageName(stageId)`，body 该 stage 桶的 steps）；当前 in_progress 的 stage 卡默认展开，已完成 stage 折叠。沿用 `planTargetIdx` 锚定。

### 数据流

```
[stage 入口] orchestrator.emit PlanUpdated{stage_id=scoping, seed} → FE: plansByStage[scoping]={seed} → 卡立刻出
[模型 update_plan] execute_plan_tool(stage=scoping)      → PlanUpdated{stage_id=scoping, real} → FE 覆盖该桶
[stage 完成 → recon] orchestrator.emit PlanUpdated{stage_id=recon, seed} → FE 新桶 → 第二张卡
```

## 风险 / 兼容

- **种子 version 哨兵**：种子用 `version=0`；模型 PlanManager 全局递增 version（>0）→ 桶内 0≠N，覆盖正常。
- **向后兼容**：`stage_id=None`（chat / 旧 run）走旧单卡；retiredPlans 旧机制保留给单卡路径。
- **ts-rs**：`just gen-types` 重生成 GeneratedAiEvent.ts 并提交（I5）。
- **不动** gate / 证据 / 授权 / stage DAG。

## 范围切分

- **Part 1（已实现）**：后端 1+2+3+4 + 前端 5+6 → 每阶段一张卡、scoping 准时出卡。
- **Part 2（已实现）**：完整 roadmap（一次性下发全 stage 大纲，未来阶段灰显占位）。

## Part 2 实现（2026-06-04）

**关键决定：不新增 AiEvent 变体**，复用 Part 1 的 `version: 0` 种子 `PlanUpdated`，避免动一长串 exhaustive match（event_dispatch / cli_json / summarizer / capture / transcript / 前端 registry）。

- **后端**（`run_executor_driven`，`execute.rs`）：DAG 投影完成后、stage loop 之前，按 `dag.nodes` 顺序为**每个** stage emit 一条 `version: 0` 且步骤 `pending` 的种子 `PlanUpdated{stage_id}`。前端据此一次性拿到完整有序大纲（`stageOrder` = DAG 节点序）。
- **前端**：
  - `setStagePlan` 去重放宽：`version === 0` 的种子**总是可覆盖**（op 起的 `pending` 种子 → 阶段入口的 `in_progress` 种子 → 真 `update_plan` v≥1 逐级取代）；只有 v≥1 的同版本重放才丢弃。
  - `StagePlanStack` 对「未开始」阶段（`version === 0` 且全部 `pending`）渲染**紧凑灰显占位行**；阶段一旦运行（入口 `in_progress` 种子或真更新）即填充为完整 `InlinePlanCard`。

**状态推导全在前端**（无需额外状态事件）：future=种子 pending；active=有 in_progress；done=全 completed。

## 后续可选（不属于 Part 1/2）

- 后端每阶段入口**重置 `PlanManager`**，使模型即便写累积清单也不串台（当前靠任务 4 prompt + 种子兜底，已够用）。
