# golish-agent-bridge / bridge_executor

> **一句话职责**：`AgentExecutor` 的 bridge 实现——把 `TaskOrchestrator` 接到 `AgentBridge` 的 LLM client：Generator/Refiner/Reporter 用一次性 completion（无工具无历史），Primary Agent 子任务用完整 agentic loop（带工具+sub-agents）；含用户意图分类。

- **类型**：目录模块（属于 crate [`golish-agent-bridge`](../golish-agent-bridge.md)）
- **路径**：`backend/crates/golish-agent-bridge/src/bridge_executor/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 Task 编排各角色（Generator/Refiner/Reporter/Primary）如何调 LLM 时
- 改用户意图分类（`classify_user_intent`/`UserIntent`）或 one-shot phase 完成逻辑时

## 职责

实现 `golish-agent-kit::task_orchestrator::AgentExecutor`，连接 orchestrator 与 `AgentBridge`。Generator/Refiner/Reporter 走 one-shot completion（`simple_completion_for_phase`，无工具无历史）；Primary Agent 子任务走完整 agentic loop。`intent` 子模块做用户意图分类。

## 公开接口

| 符号 | 说明 |
|---|---|
| `AgentExecutor` impl（trait_impl） | 把 orchestrator 接到 bridge |
| `classify_user_intent` / `UserIntent` | 用户意图分类 |
| `truncate_to_char_boundary`（`pub(crate)`） | UTF-8 安全截断 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | helper + re-export |
| `trait_impl.rs` | `AgentExecutor` 实现（各角色 LLM 调用） |
| `intent.rs` | 用户意图分类 |

## 依赖

- crate 内 `agent_bridge::AgentBridge`、`golish-agent-kit`（AgentExecutor trait）、`golish-llm-providers`、`rig`

## 注意事项 / 坑

- 角色分工：Generator/Refiner/Reporter **无工具无历史**（one-shot），Primary 才用全 loop——别给 one-shot 角色挂工具。
- 历史在 A1-3 从 golish-ai 搬来，依赖 `AgentBridge`（所以放 bridge crate 而非 agent-kit）。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-bridge bridge_executor
```
