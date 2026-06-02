# Engine v2 · P2 — metalcraft 图执行器接管 operation 流转

> 目标：把已 vendor 的 metalcraft 图引擎（`harness/graph_engine/`）从「装好没插电」变成**真正驱动 operation 阶段流转**的执行器，替掉当前游标式 `decide_transition` + `advance_target`（「永远取第一候选」），拿到 **条件分支路由** + **全量检查点/断点续跑**。
>
> 来源：`docs/design/2026-06-02-harness-vs-mainstream-gap-analysis.md` §5 P2 + 附录 C/A。
> 日期：2026-06-02。原则：增量、每步 precommit 绿、不破核心链路（增量 1-2 additive/flag-gated）。

## 背景（已核对真实代码）

- `harness/graph_engine/`：metalcraft 图引擎已 vendor（`Graph`/`Executor`/`Checkpointer`/`to_mermaid`/确定性并行），单测过，但**只在 harness 内被引用，未接 orchestrator**。
- `operation_graph.rs`：`AllowedDag` 是真正在跑的阶段拓扑（profile 投影）。base graph 12 节点 15 边，**已含 4 条 bail-to-reporting 短路边**（eas/enumeration/vuln_triage/verification → reporting）。
- `stage_transition.rs`：`decide_transition` 决定 Hold/Complete/Advance/Branch；`advance_target()` 对 `Branch` **取 `candidates.first()`** → 4 条 bail 边形同虚设（gap P2-a）。
- `drive_stage_transition`（execute.rs）：游标式驱动，读 gate outcome → 推 `operation_state` 游标 + 审批 hold/resume。

## 增量拆分

### 增量 1（本次 · additive · 零行为变更）— `harness/operation_flow.rs`
把 `AllowedDag` 编译成 metalcraft `CompiledGraph<OperationFlowState>`，用 metalcraft `Executor` + 条件边驱动**模拟**的阶段流转，证明可行 + 拿到条件路由能力。**不碰** live `drive_stage_transition`。

- `OperationFlowState`（impl `Reducer`）：`current` / `visited` / 每阶段 `StageFlowOutcome{gate_allowed, made_progress}`（seeded 输入 + applied 记录）。
- `build_operation_flow_graph(dag)`：每 stage 一个 node；单后继→静态边；多后继→**条件边**（`made_progress`→`candidates[0]` 主路；否则→`candidates.last()` bail-to-reporting）；终点→`END`。gate blocked 的 node→`Interrupt`（= 现 `Hold`，可 resume 返工）。
- `run_operation_flow(dag, seeded)`：`Executor` + `MemoryCheckpointer` 跑。
- 单测：① assessment DAG 编译成图 + `to_mermaid` ② 全程有进展走完整路径 ③ **eas 无进展→bail 到 reporting（跳过 enumeration）** ④ gate blocked→Interrupt+resume ⑤ checkpointer round-trip。

### 增量 2（下次 · flag-gated）— 让 live 流转走图
`drive_stage_transition` 增一条「图执行器」路径（`GOLISH_HARNESS_GRAPH_FLOW` flag，默认旧游标）：用增量 1 的图 + 真实 gate outcome（made_progress = 有 findings/surface）决定下一 stage，替掉 `advance_target` 的「取第一候选」。审批 hold 复用现有 `stage_entry_requires_approval`。

### 增量 3（下次）— DB-backed Checkpointer + 全量状态
实现 `Checkpointer` for `operation_state`/`stage_runs`（写 state_blob + 开/闭 stage_run 行），让杀进程后能从任意 stage resume（gap P2-b）。

### 增量 4（可选）— 节点体真跑阶段
node body 调 orchestrator 跑该 stage 的 subtasks（控制反转），Executor 全程驱动。风险最大，最后做。

## 验收
- 每增量：`just precommit` 绿 + 新单测过。
- 增量 1 不改任何 live 路径（grep 确认 `operation_flow` 仅被自身 + mod.rs 引用）。
