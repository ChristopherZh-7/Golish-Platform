# golish-events / event_coordinator

> **一句话职责**：单任务消息传递的事件协调器——把事件相关状态（序号 / frontend-ready 标志 / 事件缓冲 / pending approvals）收进一个 tokio 任务按确定序处理命令，消灭共享锁式可变状态的死锁可能。

- **类型**：目录模块（属于 crate [`golish-events`](../golish-events.md)）
- **路径**：`backend/crates/golish-events/src/event_coordinator/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改事件序号分配、frontend-ready 缓冲重放、pending approval 登记/解析时
- 改 `AgentBridge` 与协调器之间的命令协议（`CoordinatorCommand`）时
- 排查事件丢失/乱序、approval 解析竞态、协调器死锁时

## 职责

把所有事件状态集中到单个 tokio 任务（owns `event_sequence` / `frontend_ready` / `event_buffer` / `pending_approvals`），`AgentBridge` 通过 `CoordinatorHandle` 发命令，任务按确定顺序处理并经 `Arc<dyn GolishRuntime>` 发射事件。单任务模型避免共享锁死锁。

## 公开接口

| 符号 | 说明 |
|---|---|
| `EventCoordinator` | 协调器主体 + spawn + 命令循环 |
| `CoordinatorHandle` | 廉价可克隆的发送端 API（`resolve_approval` 等） |
| `CoordinatorCommand` | 命令枚举 |
| `CoordinatorState` | 状态快照 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `coordinator.rs` | `EventCoordinator` 本体 + spawn + 命令处理循环 |
| `handle.rs` | `CoordinatorHandle`（发送端） |
| `commands.rs` | `CoordinatorCommand` + `CoordinatorState` |

## 依赖

- `golish_core::events`（`AiEvent`/`AiEventEnvelope`）、`GolishRuntime`（事件发射）、`tokio`

## 注意事项 / 坑

- **单任务串行**是核心设计：别为"性能"改回共享锁可变状态（会重新引入死锁）。
- `frontend_ready` 前事件进缓冲，ready 后重放——前端晚连不会丢早期事件。
- approval 是"先 register 后 emit"（无 resolve-before-register 竞态，见 headless runner 勘探）。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-events event_coordinator
```
