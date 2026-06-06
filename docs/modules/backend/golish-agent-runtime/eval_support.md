# golish-agent-runtime / eval_support

> **一句话职责**：评测支持——给 evals 提供简化入口跑**同一条** agentic loop（确保 eval 测真实行为）：`run_eval_agentic_loop`（+ with_tools）单轮、`run_multi_turn_eval` 多轮串会话历史，外加事件流→结构化 tool calls/文件列表的提取。

- **类型**：目录模块（属于 crate [`golish-agent-runtime`](../golish-agent-runtime.md)）
- **路径**：`backend/crates/golish-agent-runtime/src/eval_support/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 evals 跑 agentic loop 的入口、配置、事件流后处理时
- 加多轮 eval 或自定义工具执行器（`CustomToolExecutor`）时

## 职责

让 evals 复用主应用同一条 loop（而非另写一套），保证测的是真实行为。`single_turn` 单轮入口、`multi_turn` 跨多 prompt 串历史、`extractors` 把捕获的事件流处理成结构化 tool calls + 文件列表 + 可读打印。

## 公开接口

| 符号 | 说明 |
|---|---|
| `run_eval_agentic_loop` / `run_eval_agentic_loop_with_tools` | 单轮 eval 入口 |
| `run_multi_turn_eval` / `MultiTurnEvalOutput` | 多轮 eval |
| `CustomToolExecutor` | 自定义工具执行器 |
| `EvalToolCall` / `EvalAgentOutput` / `EvalConfig`（types） | eval DTO |

## 关键文件

| 文件 | 作用 |
|---|---|
| `single_turn.rs` / `multi_turn.rs` | 单轮 / 多轮入口 |
| `extractors.rs` / `types.rs` | 事件流后处理 / DTO |

## 依赖

- crate 内 `agentic_loop`；`golish-events`（事件流）

## 注意事项 / 坑

- **复用真实 loop**：别为 eval 另写简化 loop（会测不到真实行为）。
- 配合 `test_utils` 的 mock LLM 跑确定性 eval。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-runtime eval_support
```
