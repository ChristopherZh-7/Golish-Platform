# golish-agent-kit / task_orchestrator

> **一句话职责**：harness 驱动的自动化任务执行——一个 Task 由 metalcraft Executor 在 profile 投影的 Operation DAG 上推进：每阶段自规划+派发 specialist、提交 StageDeliverable、过确定性 evidence gate 才前进（大阶段边界 HITL），末尾 reporter 收尾。

- **类型**：目录模块（属于 crate [`golish-agent-kit`](../golish-agent-kit.md)）
- **路径**：`backend/crates/golish-agent-kit/src/task_orchestrator/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 Task 模式编排（stage 自规划/派发/gate/前进、reporter 收尾）时
- 改 `AgentExecutor` trait、`TaskOrchestrator::run` 入口、per-stage agentic loop 时

## 职责

在 `AgentBridge` 之上编排整个 Task 生命周期 + DB 持久化，每次 agent 调用回落到 bridge。`subtask_phases` 跑 Executor 驱动的 operation loop + `execute_single_subtask`（per-stage loop + gate）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `TaskOrchestrator`（`run`） | 编排主体 + 入口 + 事件发射 |
| `AgentExecutor`（trait） | 每次 agent 调用的抽象（bridge 实现在 `golish-agent-bridge::bridge_executor`） |
| `types`（planning DTO / token usage / 执行上下文） | 编排类型 |
| `prompts` | 各阶段 prompt 模板 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `orchestrator.rs` | `TaskOrchestrator` + `run` |
| `subtask_phases/` | Executor loop + `execute_single_subtask`（per-stage gate） |
| `types.rs` / `prompts/` / `helpers.rs` | DTO/trait · prompt 模板 · 共享小函数 |

## 依赖

- crate 内 `harness`（gate/DAG）、`AgentBridge`（经 trait）；`golish-events`

## 注意事项 / 坑

- **不变量 I7/I8**：每阶段必须过 evidence gate 才前进；gate 是确定性规则，不能拿「agent 自信说完成」当通过。
- `bridge_executor`（`AgentExecutor` 实现）在 `golish-agent-bridge`（依赖 AgentBridge）；本模块只持 trait。
- graph-flow 的 `operation_state.current_stage` 表示**当前正在执行的 stage**：进入新 stage 时同步并刷新 `stage_started_at`；断线后回到同一 stage 时不能重复刷新，否则 freshness-window gate 会看不到断线前已落库的 evidence。
- `operation_state.state_blob` 是 graph checkpoint、stage_run worker resume 等多消费者共享 JSON；更新 `graph_flow` 时要 merge 保留其他 key，不能整段覆盖。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-kit task_orchestrator
```
