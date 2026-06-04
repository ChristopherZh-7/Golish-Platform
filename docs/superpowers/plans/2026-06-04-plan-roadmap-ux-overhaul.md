# Plan 路线图 UX 整改 实现计划

> **面向 AI 工作者：** 用 `.cursor/skills/executing-plans` 逐任务实现，每任务单独 commit。TDD：先写失败测试再实现。设计见 `docs/design/2026-06-04-plan-roadmap-ux-overhaul.md`。

**目标：** 修三件事——① 「Stage complete」里程碑 + 卡片阶段完成态只在**后端权威证据门 PASS** 后出现（不再用 submit 预览 / 模型自报）；② 路线图所有阶段共用一套视觉语言（统一 `StageRow`，不动 `InlinePlanCard`）；③ 加常驻吸顶进度条显示当前阶段/步骤。
**架构：** 后端在 gate 汇聚点 `consume_gate_outcome` 复用 `AiEvent::TaskProgress{status:"stage_passed"}` emit 权威信号（零 ts-rs 改动）；前端把里程碑改挂此信号 + 记 `passedStages`，新增 `StageRow` / `PlanStepRow` / `StageProgressBar` 三组件。
**技术栈：** Rust（golish-agent-kit）+ React/TS（frontend）+ Vitest + cargo nextest。

---

## 文件结构

**后端**
- `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs` — 改：`consume_gate_outcome` 在 PASS 时 emit `TaskProgress{status:"stage_passed", message:stage_id}`。
- `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute_harness_loop_tests.rs` — 加测：PASS 产生 stage_passed 事件。

**前端**
- `frontend/components/AIChatPanel/hooks/useAiChatEvents.ts` — 改：删 submit-preview 触发；`task_progress` 分支处理 `stage_passed`。
- `frontend/store/slices/conversation.ts` — 加：`passedStages` + `markStagePassed`（或 session slice，取与 `setStagePlan` 同源的 sessionId）。
- `frontend/components/AIChatPanel/PlanStepRow.tsx` — 新：从 `InlinePlanCard` 抽出的共享步骤行（`StepIcon` + `StepRow`）。
- `frontend/components/AIChatPanel/InlinePlanCard.tsx` — 改：import `PlanStepRow`（纯重构）。
- `frontend/components/AIChatPanel/StageRow.tsx` — 新：统一阶段行/卡。
- `frontend/components/AIChatPanel/StagePlanStack.tsx` — 改：用 `StageRow` 取代内联两套样式。
- `frontend/components/AIChatPanel/StageProgressBar.tsx` — 新：吸顶当前进度条。
- `frontend/components/AIChatPanel/AIChatPanel.tsx` — 改：滚动容器内挂 `StageProgressBar`。
- `frontend/components/AIChatPanel/hooks/useTaskPlanState.ts` — 改：暴露 `passedStages`。

---

## Part A · 完成态绑权威证据门

### 任务 A1 · 后端在 gate PASS 时 emit stage_passed

- **文件**：`execute.rs`、`execute_harness_loop_tests.rs`
- **步骤**：
  1. 在 `consume_gate_outcome` 现有 `if outcome.gate_allowed { … }` 块内（紧接 `harness_evidence.insert` 之后）加：

```rust
self.emit(AiEvent::TaskProgress {
    task_id: task_id.to_string(),
    status: "stage_passed".to_string(),
    message: outcome.gated_stage.as_str().to_string(),
});
```

  2. 测试：在 `execute_harness_loop_tests.rs` 用既有 `pass(stage)` helper 驱动一次 `consume_gate_outcome`，断言捕获到的事件含 `AiEvent::TaskProgress { status, message, .. }` 且 `status == "stage_passed"` && `message == "scoping"`（用现有事件捕获 sink，参考同文件 `waiting_approval` 断言模式）。
- **验证**：`cd backend && cargo nextest run -p golish-agent-kit -E 'test(execute_harness_loop)'` → 新测试通过、原测试不回归。
- **提交**：`feat(harness): emit stage_passed TaskProgress when the evidence gate passes`

### 任务 A2 · 前端把里程碑改挂 stage_passed

- **文件**：`useAiChatEvents.ts`
- **步骤**：
  1. 删除 `tool_result` 分支里 `if (event.tool_name === "submit_stage_deliverable" && submitResultAccepted(event.result)) { … }` 整段（约 211–228），以及只服务它的 `submitStageByRequestRef`、`rememberSubmitStage` 调用点、`submitResultAccepted`、`rememberSubmitStage` 定义（grep 确认无其它引用后删）。
  2. 在 `task_progress` 事件 case（emit `TaskProgress` 的处理处）新增：

```ts
if (event.status === "stage_passed") {
  const stageId = event.message;
  if (stageId && lastStageRef.current.get(convId) !== stageId) {
    lastStageRef.current.set(convId, stageId);
    store.addConversationStageMarker(convId, {
      kind: "stage_completed",
      label: `Stage complete: ${prettyStageName(stageId)}`,
      status: "finished",
    });
    const sid = useStore.getState().conversations[convId]?.aiSessionId;
    if (sid) store.markStagePassed(sid, stageId);
  }
}
```

  （`prettyStageName` 已从 `../StageMarker` import；`lastStageRef` 已存在。）
- **验证**：`cd frontend && pnpm --silent typecheck` → exit 0；`pnpm vitest run components/AIChatPanel/hooks` 若有相关测试则更新断言（submit accepted 不再产 marker；stage_passed 产 marker）。
- **提交**：`fix(plan-ui): drive Stage-complete milestone off the authoritative gate, not the submit preview`

### 任务 A3 · Store 记 passedStages

- **文件**：`store/slices/conversation.ts`（或 session slice，须与 `setStagePlan` 写的同一 sessionId 维度一致）、`store/store-types.ts`（`Session` 类型）
- **步骤**：
  1. `Session` 加 `passedStages?: string[]`。
  2. 加 action：

```ts
markStagePassed: (sessionId: string, stageId: string) =>
  set((state) => {
    const s = state.sessions[sessionId];
    if (!s) return;
    if (!s.passedStages) s.passedStages = [];
    if (!s.passedStages.includes(stageId)) s.passedStages.push(stageId);
  }),
```

  3. 测试：`store` 单测——`markStagePassed` 幂等、写对 session。
- **验证**：`pnpm --silent typecheck`；`pnpm vitest run store` 相关用例绿。
- **提交**：`feat(store): record authoritatively passed stages`

---

## Part B · 统一 StageRow 组件

### 任务 B1 · 抽 PlanStepRow（纯重构）

- **文件**：新增 `PlanStepRow.tsx`；改 `InlinePlanCard.tsx`
- **步骤**：
  1. 把 `InlinePlanCard.tsx` 里的 `StepIcon` + `StepRow` + `FailureKindBadge` 原样移到 `PlanStepRow.tsx` 并 `export`。
  2. `InlinePlanCard.tsx` 改为 `import { StepRow } from "./PlanStepRow";` 删除本地定义。
- **验证**：`pnpm --silent typecheck`；`pnpm vitest run components/AIChatPanel/InlinePlanCard.test.tsx` → 原断言全绿（行为不变）。
- **提交**：`refactor(plan-ui): extract shared PlanStepRow from InlinePlanCard`

### 任务 B2 · StageRow + StagePlanStack 改用它

- **文件**：新增 `StageRow.tsx`；改 `StagePlanStack.tsx`、`useTaskPlanState.ts`
- **步骤**：
  1. `useTaskPlanState.ts`：从 store 选 `passedStages`（与 `storePlansByStage` 同源），并入返回值；`StagePlansViewModel` 加 `passedStages: string[]`（`./TaskPlan` 类型）。
  2. `StageRow.tsx`：props `{ stageId, plan, passed }`。统一锚定 `[状态图标] {prettyStageName(stageId)} · {completed}/{total}`：
     - `passed` → ✅ 绿勾、折叠、可点开（展开用 `PlanStepRow` 列 steps）。
     - 有 `in_progress` 且未 passed → ⟳ spinner、默认展开、列 `PlanStepRow`。
     - 全 pending 且 version 0（未来）→ 空心圆 + 灰显单行，无 steps。
  3. `StagePlanStack.tsx`：删 `isFutureStage` 内联渲染与 `InlinePlanCard` 用法，改为 `stageOrder.map(stageId => <StageRow stageId plan={plansByStage[stageId]} passed={passedStages.includes(stageId)} />)`。
- **验证**：`pnpm --silent typecheck`；新增 `StageRow.test.tsx`（四态渲染：future/active/passed/done-not-passed）；`pnpm vitest run components/AIChatPanel`。
- **提交**：`feat(plan-ui): unify roadmap stages into one StageRow visual language`

---

## Part C · 吸顶当前进度条

### 任务 C1 · StageProgressBar + 挂载

- **文件**：新增 `StageProgressBar.tsx`；改 `AIChatPanel.tsx`
- **步骤**：
  1. `StageProgressBar.tsx`：props `{ stagePlans, passedStages, onJump }`。推导：
     - `currentStageId` = `stageOrder` 中第一个 `!passedStages.includes(id)` 且其 plan 有 in_progress 步骤的；退而取第一个未 passed 的。
     - `currentStep` = 该阶段 plan 的 in_progress 步骤（无则取首个 pending）。
     - 渲染：`⟳ {prettyStageName(currentStageId)} · {idx+1}/{stageOrder.length}` + 若有 step：` — {done+1}/{total}: {step.step}`。`sticky top-0 z-10` 一行细条；点击 `onJump()`。
  2. `AIChatPanel.tsx`：在 `:290` 滚动容器内、`messages.map` 之前插入：

```tsx
{stagePlans && (
  <StageProgressBar
    stagePlans={stagePlans}
    passedStages={stagePlans.passedStages}
    onJump={() => {
      const id = storePlanMessageId; // 已有；或 messagesEndRef 反查 planTargetIdx
      document.getElementById(`msg-${id}`)?.scrollIntoView({ behavior: "smooth", block: "start" });
    }}
  />
)}
```

     （`stagePlans` 来自 `useTaskPlanState`，组件已用它。若消息节点无 `id={`msg-${m.id}`}`，在 `MessageBlock` 包裹处补一个锚 id。）
- **验证**：`pnpm --silent typecheck`；新增 `StageProgressBar.test.tsx`（推导当前阶段/步骤 + 点击回调）；`pnpm vitest run components/AIChatPanel`；手动：`just dev` 滚动聊天时进度条吸顶不动、内容随阶段更新。
- **提交**：`feat(plan-ui): add sticky StageProgressBar (you-are-here)`

---

## 收口

### 任务 Z · 验证 + 收尾

- **步骤**：
  1. `just gen-types` → `git status` 确认 `GeneratedAiEvent.ts` **无**漂移（复用 TaskProgress，预期无变更）。
  2. `cd frontend && just check-fe && just test-fe`。
  3. `cd backend && cargo nextest run -p golish-agent-kit && cargo clippy -p golish-agent-kit --all-targets -- -D warnings && cargo fmt -p golish-agent-kit --check`。
  4. `code-audit` 收口（diff 范围 / 调用链 / 影响面 / 是否漏改）。
  5. 手动 E2E：task『搞一下 example.com』→ ① scoping 未过门前**不**显示「Stage complete」；门过后才显示 + 卡变绿。② 路线图里外样式一致。③ 滚到下面仍有吸顶条显示当前阶段/步骤。
  6. 更新 `feature_list.json`（本特性条目 → passing/blocked + evidence）、`agent-progress.md`。
- **验证**：受影响范围全绿；E2E 三点符合预期。
- **提交**：`chore(plan-ui): audit + verify roadmap UX overhaul`

---

## 自检

- **规格覆盖**：问题①→Part A（A1 后端信号 / A2 里程碑改挂 / A3 passedStages + B2 卡片绿绑 passed）；问题②→Part B（B1 抽行 / B2 统一 StageRow）；问题③→Part C（C1 吸顶条）。
- **类型一致**：`passedStages: string[]` 贯穿 store(`Session`)→`useTaskPlanState`→`StagePlansViewModel`→`StageRow`/`StageProgressBar`；`markStagePassed(sessionId, stageId)` 签名各处一致。
- **占位符扫描**：各任务含真实文件路径 + 真实代码块 + 真实命令，无 TODO/待定。
- **向后兼容**：复用 TaskProgress（旧前端忽略新 status）；`passedStages` 缺省=旧行为；`InlinePlanCard` 聊天单卡不动；吸顶条仅 task-mode。
- **风险**：A1 emit 点在 PASS 分支，确保不与两级审批 hold 冲突（hold 是 PASS 之后的 cross-phase 决策，stage_passed 表"本阶段证据门已过"语义正确）。
