# 阶段内 Agent-Todo 执行 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现此计划。每个任务单独 commit。无 flag、无回滚分支（测试阶段，直接改完）。

**目标：** 把 task 模式"阶段内"的执行从 generator 产 JSON 子任务 + 固定子任务循环，改成"主 agent 在每个 harness 阶段内用 `update_plan` 自管 todo、按需派 sub-agent、最后 `submit_stage_deliverable` 过 gate"。

**架构：** harness 外层骨架（stage DAG 序列 / 逐阶段工具授权 / gate / 证据）**全部保留**。只替换每个 stage 的"体"——`run_stage_subtasks` 内部从"调 `generate_stage_plan` + 逐子任务 `execute_single_subtask`"改成"跑一个阶段级 agentic loop（depth-0 primary），它用 `update_plan` 决定要不要 todo、派 sub-agent、提交 StageDeliverable"。顺带修两条根因：① 入口不再能绕过 harness 直接派 pentester；② scoping 等 confirm-only 阶段加确定性交付物兜底，杜绝 `content_len=0` 死锁。

**技术栈：** Rust（golish-agent-kit / golish-agent-bridge / golish-agent-runtime workspace）+ cargo nextest。

---

## 背景：当前两条根因（已定位，证据见会话）

- **① 入口绕过**：lead turn（`golish-agent-app/src/ai/commands/core/chat.rs:90-104`）让主模型在 `start_operation`（→harness）和"直接答 / 直接派 `sub_agent_pentester`"之间选；弱模型选了直接派 pentester → harness 没启动 → 无 scoping。
- **② scoping gate 死锁**：`run_stage_subtasks`（`execute.rs:602`）对 confirm-only 阶段合成 1 个子任务 → `execute_isolated` 返回空 + agent 未调 `submit_stage_deliverable` → gate（`execute.rs:1357`）拿到 `content_len=0` → fail-closed BLOCK ×3 → `Interrupted`。

## 已核实的关键事实（决定本计划接线）

1. **graph-flow 是 task 默认路径**：`stage_mode_enabled()` 与 `graph_flow_enabled()` 均默认 ON（`harness/mod.rs:127`、`harness/operation_flow.rs:39`）。stage 序列由 metalcraft `Executor` 驱动（`execute.rs:489-571`），每个 stage 体 = `run_stage_subtasks`。
2. **`update_plan` 当前不在 task primary 工具集**：`TaskModePolicy::primary_tools`（`golish-agent-runtime/src/execution_mode/modes/task.rs:41`）用 `static_groups::none()`，而 `update_plan` 属 static 组（`execution_mode/prompt_render.rs:68`）。→ 必须显式给 primary 加 `update_plan`。
3. **sub-agent 阶段授权已具备**：`sub_agent_call.rs:73` 用 `ctx.harness_stage` 建 `StageToolGuard` + zero-scan 隐藏（`:111`），depth-1 已被 stage 授权覆盖（设计 `docs/design/2026-06-02-stage-tool-whitelist-enforcement.md`）。→ 只需保证阶段 loop 运行时 `harness_active_stage` 已发布（`trait_impl.rs:166`）。
4. **deliverable 侧信道已具备**：`submit_stage_deliverable` 写 `harness_last_deliverable`（`trait_impl.rs:169` reset、`:179-196` 回灌为 ```json）。

---

## 设计决定（已与用户拍板 · 2026-06-04）

- **D1 入口行为（修 ①）= 方案 B（已选）**：**彻底去掉 lead 决策 turn**。task 模式进来**永远**直接进 harness（scoping 起），不再有"主模型在 start_operation vs 直接派 sub-agent 之间选"这一步——从根上消除 ① 的绕过路径。casual 闲聊交给 chat 模式。
  - 弃用 `start_operation` 作为入口闸（task 总是 start operation）；`StartOperationTool` 与 lead-turn 相关分支退役。
- **D2 ② 兜底 = 混合（已选）**：按阶段类型分治——
  - **确认型阶段**（`spec.allowed_tool_types.is_empty()`，如 scoping / reporting）：agent 没产 deliverable 时，**后端确定性合成最小 StageDeliverable**（scoping 的 `scope_status` 取自已注册 targets/scope）直接喂 gate → 永不死锁。
  - **有真实产出的阶段**（recon / enumeration / vuln_triage…）：**不自动合成**（findings 不能造假）；走"loop 内强制必须 submit + fail-closed BLOCK 重试 N 轮"，仍失败则 BLOCK（让引擎 Interrupt 返工，不假过）。

> 下文任务按 **D1=B + D2=混合** 编写。

---

## 文件结构（创建/修改 + 职责）

| 文件 | 改动职责 |
|---|---|
| `golish-agent-runtime/src/execution_mode/modes/task.rs` | primary 暴露 `update_plan`（阶段内主 agent 自管 todo）；入口 lead turn 收窄派发工具（D1=A） |
| `golish-agent-kit/src/task_orchestrator/types.rs` | `AgentExecutor` trait 加 `run_stage()`，标记 `generate_stage_plan` 为本路径不再用 |
| `golish-agent-bridge/src/bridge_executor/trait_impl.rs` | 实现 `run_stage()`：注入 stage charter + "可选 todo / 派 sub-agent / 末尾 submit" 指令，跑一个 depth-0 agentic loop，回灌 deliverable |
| `golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs` | 重写 `run_stage_subtasks`：删 `generate_stage_plan`+子任务循环，改调 `run_stage()` → gate → ② 确定性兜底 |
| `golish-agent-kit/src/task_orchestrator/prompts/mod.rs` | 新增 `stage_execution_prompt()`（阶段内主 agent 指令）；保留 `stage_charter` |
| `golish-agent-app/src/ai/commands/core/chat.rs` | D1：入口 lead turn 工具收窄 / 或去掉 lead turn（D1=B） |
| `golish-agent-kit/src/harness/...`（gate 兜底） | ② 确定性最小 deliverable 合成（confirm-only 阶段） |

---

## 任务分解（逐步、可单测、频繁 commit）

### 任务 1 · 给 task primary 暴露 `update_plan`
- **文件**：`golish-agent-runtime/src/execution_mode/modes/task.rs`
- **步骤**：在 `primary_tools` 的 `ToolSelection` 里显式允许 `update_plan`（最稳妥：保留 `static_groups::none()`，新增一个 `allow_overrides`/在 bridge 或 runtime 选择里点名 `update_plan`；若无 allow 机制则启用含 `update_plan` 的最小 static 子组）。同步去掉 subtask 的 `deny_overrides:["update_plan"]` 是否保留的判断（保留——只有 primary 管 plan）。
- **验证**：`cargo nextest run -p golish-agent-runtime -E 'test(execution_mode::modes::task)'`；新增断言 `task_primary_exposes_update_plan`。
- **提交**：`feat(task-mode): expose update_plan to depth-0 primary`

### 任务 2 · `AgentExecutor` 加 `run_stage`
- **文件**：`golish-agent-kit/src/task_orchestrator/types.rs`
- **步骤**：trait 加
  ```rust
  async fn run_stage(
      &self,
      stage: crate::harness::StageKind,
      exec_ctx: &ExecutionContext,
      op_max_authz: Option<crate::harness::AuthorizationLevel>,
  ) -> anyhow::Result<AgentResult>;
  ```
  返回的 `content` 已含（或已回灌）StageDeliverable，供 gate 解析。
- **验证**：`cargo check -p golish-agent-kit`。
- **提交**：`feat(harness): add AgentExecutor::run_stage contract`

### 任务 3 · 阶段执行 prompt
- **文件**：`golish-agent-kit/src/task_orchestrator/prompts/mod.rs`
- **步骤**：加 `stage_execution_prompt(stage_id)`：内容指令——「你正处于 `{stage_id}` 阶段。先判断本阶段是否需要拆 todo：简单/确认型阶段可直接产交付物；复杂阶段先调 `update_plan` 列 2-5 条 todo，再按需派 `sub_agent_*` 完成每条，逐条标完成。本阶段完成后**必须**调 `submit_stage_deliverable` 提交结构化 StageDeliverable（含 scope_status/claims/evidence_refs），只引用本 run 真实 evidence id。不要执行其它阶段的事。」
- **验证**：`cargo test -p golish-agent-kit -E 'test(prompts::stage_execution)'`（断言含 update_plan / submit_stage_deliverable 关键词）。
- **提交**：`feat(prompts): stage_execution_prompt for agent-todo stage body`

### 任务 4 · bridge 实现 `run_stage`
- **文件**：`golish-agent-bridge/src/bridge_executor/trait_impl.rs`
- **步骤**：实现 `run_stage`：
  1. 组 prompt = orchestrator base + `stage_charter(spec)` + `stage_execution_prompt(stage_id)` +（上游 evidence handoff，复用现有 `render_inherited_handoff`）。
  2. 发布侧信道：`harness_active_stage = Some(stage)`、`harness_active_authz`、`harness_last_deliverable = None`（同 `execute_subtask:166-169`）。
  3. `content = self.bridge.execute_isolated(&prompt).await?`（这一 loop 内主 agent 有 update_plan + dispatch + submit）。
  4. 回灌：若 `harness_last_deliverable` 有值且 content 无 ```json，则 append（复用 `:179-196` 逻辑）。
  5. 返回 `AgentResult::with_usage(content, ...)`。
- **验证**：`cargo check -p golish-agent-bridge`；`cargo nextest run -p golish-agent-bridge`。
- **提交**：`feat(bridge): run_stage executes one agent-todo loop per stage`

### 任务 5 · 重写 `run_stage_subtasks`（删 generator/子任务循环）
- **文件**：`golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`（602-781）
- **步骤**：整段替换为：
  1. 设 `exec_ctx.harness_stage = Some(stage)` + `harness_authz`（保留现 676-683 的 intent 分类）。
  2. `let result = executor.run_stage(stage, exec_ctx, op_max_authz).await`；失败→记录并落 BLOCK。
  3. `let (gated, outcome) = apply_harness_gate_hook(&synthetic_planned(stage), exec_ctx, result.content)`（用一个 stage-tagged 占位 planned 以复用现有 gate hook）。
  4. `enforce_evidence_existence/kinds/freshness`（保留 210-212 的证据校验）。
  5. `consume_gate_outcome` / 累加 `stage_outcome_acc`。
  6. 把结果 push 进 `exec_ctx.completed_results` + emit `SubtaskCompleted`（保持 UI/handoff）。
  - **删除**：`generate_stage_plan` 调用、`indices` 子任务循环、`synthesize_stage_subtask` 多步逻辑（confirm-only 收敛逻辑由阶段 prompt + 兜底替代）。
- **验证**：`cargo nextest run -p golish-agent-kit -E 'test(task_orchestrator)'`。
- **提交**：`refactor(harness): stage body = run_stage + gate (drop generator subtask loop)`

### 任务 6 · ② 兜底（D2=混合：确认型自动合成 / 产出型强制+BLOCK）
- **文件**：`golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`（任务 5 之后的 gate 处）
- **步骤**：`apply_harness_gate_hook` 前，若 `content` 解析不出 deliverable 且 stage 是 confirm-only（`spec.allowed_tool_types.is_empty()`），用 operation 的 scope/targets 合成最小 `StageDeliverable`（`scope_status` 来自已注册 targets / scope），喂给 gate；非 confirm-only 阶段仍走 fail-closed BLOCK + 一次 `run_stage` 重试（带 correction）。
- **验证**：新增 `cargo nextest run -p golish-agent-kit -E 'test(harness::scoping_fallback)'`：模拟空 content → scoping gate PASS（拿到合成 deliverable），enumeration 空 content → 仍 BLOCK。
- **提交**：`fix(harness): deterministic minimal deliverable for confirm-only stages (kill content_len=0 deadlock)`

### 任务 7 · 去掉 lead 决策 turn，task 直接进 harness（D1=B，修 ①）
- **文件**：`golish-agent-app/src/ai/commands/core/chat.rs`
- **步骤**：在 task 模式入口（`chat.rs:85-186` 那段 lead turn + `pending_plan_request` 分支）移除"先跑 lead turn 让模型决定 start_operation/直答"的逻辑：task 消息进来**直接**构造 task_input 并走 `execute_task_mode`（`chat.rs:189-193` 的 `TaskOrchestrator::run`）→ harness 从 scoping 起。`LEAD_AGENT_PROMPT`（`chat.rs:252`）与 `pending_plan_request` 读取分支删除；`StartOperationTool` 注册从 task 入口摘除（保留文件，避免误删被别处引用——先 grep 确认无其它消费方再决定是否删类）。
  - 注意：casual/闲聊仍由 **chat 模式**承接（不受影响）；本改只动 task 模式入口。
- **验证**：`cargo nextest run -p golish-agent-runtime -p golish-agent-app`；新增/改断言：task 入口不再调 lead turn、不暴露 `sub_agent_pentester` 给入口；手动 E2E：task『搞一下 example.com』→ backend.log 直接出现 `entering stage stage=scoping`，无"委托 pentester"绕过。
- **提交**：`fix(task-mode): drop lead decision turn; task always enters harness at scoping`

### 任务 8 · 清理 + 收口
- **步骤**：移除 graph-flow 路径下已死的 `generate_stage_plan` / `ensure_submit_terminal` / `synthesize_stage_subtask`（若无其它调用方）；`code-audit` 收口；`just precommit`。
- **验证**：`just precommit` 全绿；手动 E2E：`just dev` → task『搞一下 example.com』→ 观察 backend.log 依次 `entering stage scoping → update_plan?/submit → gate PASS → entering stage target_intel ...`，不再 `content_len=0`。
- **提交**：`chore(harness): cleanup dead generator stage path + audit`

---

## 自检

- **规格覆盖**：① 入口绕过→任务 7；② scoping 死锁→任务 5+6；update_plan 接阶段内→任务 1-5；保留 harness/gate/证据/授权→任务 5 复用 gate、任务 4 复用侧信道、事实 #3 sub-agent 授权保留。
- **类型一致**：`run_stage` 签名在任务 2 定义、任务 4 实现、任务 5 调用，三处一致。
- **风险**：阶段 loop 用 `execute_isolated`（无嵌套子任务持久化）——`SubtaskCompleted`/plan 事件需在任务 5 手动 emit 以保 UI；D1/D2 选项不同则任务 6/7 有差异（已标注）。
