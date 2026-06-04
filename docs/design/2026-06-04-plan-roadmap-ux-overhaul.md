# Plan 路线图 UX 整改（设计）

> 状态：proposed · 2026-06-04 · 关联 `2026-06-04-per-stage-plan-cards.md`（Part 1/2 已落地）
> 根因诊断来源：同日会话（MCP-agent-1，DISPATCH off，用户逐条驱动 + 3 张实操截图）

## 背景 / 三个问题

Per-stage plan 卡（设计 `2026-06-04-per-stage-plan-cards.md` 的 Part 1/2）落地后，用户在实操中发现 3 个问题：

1. **【功能】完成态没绑证据门**
   「Stage complete」绿旗里程碑由 `submit_stage_deliverable` 工具的**预览**结果触发——前端 `components/AIChatPanel/hooks/useAiChatEvents.ts:214` 在 `tool_result.status === "accepted"` 时就画里程碑。但这个 `accepted` 来自 `golish-agent-app/src/ai/harness_submit_tool.rs` 的**预览门**（仅校验结构合法 + 引用 evidence id 是否存在；无 evidence_repo 时连存在性都不查）。**真正权威的证据门**在阶段收口时才跑——`golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs::consume_gate_outcome` 的 `outcome.gate_allowed`，它还会校验 evidence kinds / freshness / min invocations / surface coverage。
   截图实证：scoping 阶段模型「no tools need to be run, I'll compile and submit directly」零证据，submit 后仍弹「Stage complete: Scoping」。卡片的"完成/绿勾"同理来自模型 `update_plan` 自报的 `completed === total`，与证据门无关。

2. **【样式】里外不搭**
   `StagePlanStack` 对**当前阶段**用 `InlinePlanCard`（带边框盒子，headline = 「X/Y tasks done」+ 展开箭头），对**未来阶段**用扁平细行（空心圆 + 阶段名，低透明度）。两套视觉语言叠在一起：未来行以"阶段名"领衔，当前盒子以"进度计数"领衔；一个轻细单行、一个厚重带框盒子。`InlinePlanCard` 本是为聊天流独立 plan 卡设计的，且被 `MessageBlock.tsx:190` 复用，不能直接改它。

3. **【交互】无常驻「你在这」指示**
   整张路线图锚定在它首次出现的消息上（`planMessageId` / `useTaskPlanState.planTargetIdx`），内联在聊天流。agent 持续向下输出（叙述 / 工具调用 / 委托）后，卡片被顶出视口；用户不滚到最上面就不知道"现在在哪个阶段、第几步"。

## 目标

- 完成态（里程碑 + 卡片阶段级绿勾）只在**后端权威证据门 PASS** 后出现；不再用 submit 预览 / 模型 `update_plan` 自报。
- 路线图所有阶段（未来 / 进行中 / 已完成）共用一套视觉语言；**不动** `InlinePlanCard` 的聊天流单卡用法。
- 加一个常驻吸顶进度条，随时显示当前阶段 + 步骤；点击展开 / 定位完整路线图。
- 向后兼容 chat / 非 harness 单卡。**不动** harness 骨架（DAG / gate / 证据 / 授权）。

## 设计

### Part A · 完成态绑权威证据门（功能）

**后端**

`execute.rs::consume_gate_outcome(task_id, outcome)` 是两条 gate 路径（legacy 逐 subtask 与 graph-driven）的唯一汇聚点，已在此处 `tracing` 记录每个 gate PASS/BLOCK 决策。在 `outcome.gate_allowed == true` 时，额外 emit 一个确定性"阶段已过门"信号：

- **复用 `AiEvent::TaskProgress`**（沿用 Part 2「不新增 AiEvent 变体」的决定，避免动一长串 exhaustive match：event_dispatch / cli_json / summarizer / capture / transcript / 前端 registry）：

```rust
// execute.rs::consume_gate_outcome，gate_allowed 分支内
self.emit(AiEvent::TaskProgress {
    task_id: task_id.to_string(),
    status: "stage_passed".to_string(),
    message: outcome.gated_stage.as_str().to_string(), // 载 stage_id
});
```

`TaskProgress { task_id, status, message }` 字段不变 → **零 IPC 改动、零 ts-rs 漂移**；`message` 载 stage_id 与现有 `status:"waiting_approval"` 复用 TaskProgress 的模式一致。

**前端**

- `useAiChatEvents.ts`：
  - **移除** `tool_result` 分支里「`submit_stage_deliverable` + `submitResultAccepted` → `stage_completed` marker」整段（约 211–228 行）及配套 `submitStageByRequestRef` / `rememberSubmitStage` / `submitResultAccepted`（若无其它引用一并清理）。
  - 在 `task_progress` 事件分支新增：`status === "stage_passed"` → `stageId = event.message` → 去重（沿用 `lastStageRef`）后 `store.addConversationStageMarker(convId, { kind:"stage_completed", label:` + "`Stage complete: ${prettyStageName(stageId)}`" + ` , status:"finished" })`，并 `store.markStagePassed(sessionId, stageId)`。
- Store（`store/slices/conversation.ts` 或 session slice）：`Session` 增 `passedStages?: string[]`；新增 `markStagePassed(sessionId, stageId)` 幂等写入。
- 卡片**阶段级**完成判定改看 `passedStages.includes(stageId)`，**不**看 `completed === total`。模型 `update_plan` 的 todo 勾仍照常显示（步骤级进度），但"本阶段是否完成"以权威门为准。

> 语义：gate PASS = 证据已被确定性门校验通过 = 用户要的「拿到证据」。两级审批的 cross-phase hold 是另一回事，不影响"本阶段证据门已过"的判定。

### Part B · 统一 StageRow 组件（样式）

- 新增 `components/AIChatPanel/StageRow.tsx`，`StagePlanStack` 改用它；**不动** `InlinePlanCard`（`MessageBlock` 聊天单卡照旧）。
- 抽出 `StepRow` / `StepIcon`（现内联于 `InlinePlanCard.tsx`）到 `components/AIChatPanel/PlanStepRow.tsx` 共享，`InlinePlanCard` 与 `StageRow` 都 import（DRY，纯重构，行为不变）。
- **每个阶段统一锚定**：`[状态图标]  阶段名  ·  X/Y`（X/Y 为名字后小号副信息，不再当大标题）。状态图标统一词汇：
  - 未来（未开始）= 空心圆（灰）
  - 进行中 = ⟳ spinner（accent）
  - 已过门（`passedStages` 命中）= ✅ 绿勾
  - blocked / waiting_approval = 琥珀
- 展开规则：当前阶段（in_progress 且未过门）默认展开，下挂该阶段 todo（`PlanStepRow`）；未来阶段 = 单行灰显（无 todo）；已过门阶段 = 折叠绿勾，可点开复看。
- 阶段名是 headline（与未来行一致）→ 里外视觉统一。

### Part C · 吸顶当前进度条（交互）

- 新增 `components/AIChatPanel/StageProgressBar.tsx`。
- 挂载点：`AIChatPanel.tsx:290` 的消息滚动容器 `<div ref={messagesContainerRef} className="flex-1 overflow-y-auto …">` 内，作为**第一个子元素**，`className="sticky top-0 z-10 …"`，仅当 `stagePlans` 存在时渲染（chat 模式不渲染）。
- 内容：`⟳ {当前阶段名} · {idx+1}/{stageOrder.length} — step {done+1}/{该阶段 total}: {当前 in_progress 步骤标题}`。点击 → 用 `planMessageId` 定位滚动到内联 `StagePlanStack`。
- 推导（全前端，无新事件）：当前阶段 = `stageOrder` 中第一个"有 in_progress 步骤或未过门"的阶段；当前步骤 = 该阶段 plan 的 in_progress 步骤。
- 复用 Part B 的状态图标 + `prettyStageName`。

## 范围切分 / 顺序

- **Part A**（功能 · 后端 emit + 前端 event/store · 最重要）
- **Part B**（纯前端样式 · 统一卡 + 抽 PlanStepRow）
- **Part C**（纯前端交互 · 吸顶条 · 依赖 B 的状态图标/推导）

建议顺序 **A → B → C**；B / C 为纯 UI，可在 A 之后并行。三者均可独立交付、独立验证。

## 风险 / 兼容

- 复用 `TaskProgress`：零新变体、零 ts-rs 漂移；`status:"stage_passed"` 是新字符串值，旧前端忽略（向后兼容）。
- 不动 `InlinePlanCard` → chat 单卡零回归；抽 `PlanStepRow` 为纯重构（行为不变 + 现有 `InlinePlanCard.test` 守门）。
- `passedStages` 为新增可选字段，缺省 = 旧行为（无阶段过门 → 不显示 complete）。
- 吸顶条只在 task-mode（有 `stagePlans`）出现，chat 模式不渲染、不占位。
- **不动** harness DAG / gate / 证据 / 授权 / stage spec。
