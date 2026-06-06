# golish-agent-kit / loop_detection

> **一句话职责**：循环检测与保护——`ExecutionMonitor` 防无限循环/失控：跟踪每请求总 turn 数、每 turn 工具调用数（内循环）、相同参数的重复工具调用。

- **类型**：目录模块（属于 crate [`golish-agent-kit`](../golish-agent-kit.md)）
- **路径**：`backend/crates/golish-agent-kit/src/loop_detection/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改循环保护阈值（max turn / max tool loops / max repeated tool calls）或检测逻辑时
- agent 报 LoopWarning/LoopBlocked/MaxIterationsReached 相关时

## 职责

`ExecutionMonitor` 跟踪并拦截失控行为：总 turn 数、每 turn 工具调用数、相同参数重复调用次数。超限发 `AiEvent::LoopWarning`/`LoopBlocked`/`MaxIterationsReached`（见 `golish-core::events`）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `ExecutionMonitor` | 循环监视器 |
| `LoopProtectionConfig`（`max_tool_loops` / `max_repeated_tool_calls` / max turns…） | 保护配置 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `LoopProtectionConfig` + re-export |
| `monitor.rs` | `ExecutionMonitor` 检测逻辑 |

## 依赖

- crate 内；`serde`（config）

## 注意事项 / 坑

- 阈值过松会放任失控（烧 token/钱）、过紧会误杀合法长任务；改阈值要兼顾 Task 模式的多 turn。
- 对应前端的 LoopWarning/LoopBlocked 事件（`golish-core::events` + `cli_json::loop_guard`）。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-kit loop_detection
```
