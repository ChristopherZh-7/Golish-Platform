# golish-events

> **一句话职责**：AI 事件协调与 transcript 系统——`DomainEvent` 统一事件枚举 + `EventCoordinator` 单任务消息协调器 + `TranscriptWriter` 把事件落成 JSONL。

- **类型**：crate（Layer 2/3）
- **路径**：`backend/crates/golish-events/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 AI 事件流、事件协调（单任务 actor）、transcript 落盘、op_trace/时间线时
- 排查「transcript 落在哪 / 事件丢了」时

## 职责

统一所有领域事件并协调分发。`EventCoordinator` 是单任务消息传递协调器；`TranscriptWriter` 把 AI 事件持久化为 JSONL；`op_trace` 负责操作轨迹/时间线/manifest。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `DomainEvent`（+ `IndexerEvent` / `PentestEvent` / `SidecarEvent`） | 顶层事件枚举 |
| `EventCoordinator` / `CoordinatorHandle` / `CoordinatorState` | 事件协调器与句柄 |
| `TranscriptWriter` / `TranscriptEvent` / `transcript_path` / `read_transcript` | transcript 读写 |
| `op_trace`：`build_manifest` / `collect_records` / `render_timeline` / `resolve_transcript_base` | 轨迹/时间线 |

## 依赖

- **内部**：`golish-core`

## 被谁依赖 / 改动影响面

`golish-agent-kit`、`golish-agent-runtime`、`golish-agent-bridge`、`golish-agent-app`。是 agent 层观测/回放的基础。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `event_coordinator/` | 单任务事件协调器 | [→](golish-events/event_coordinator.md) |
| `op_trace/` | 操作轨迹 / 时间线 / manifest | [→](golish-events/op_trace.md) |
| `transcript/` | JSONL transcript 读写与摘要 | [→](golish-events/transcript.md) |

## 关键文件

| 文件 | 作用 |
|---|---|
| `domain_event.rs` | `DomainEvent` 及子事件枚举 |

## 注意事项 / 坑

- transcript 落盘路径解析顺序见 `op_trace::resolve_transcript_base`：`VT_TRANSCRIPT_DIR` 覆盖 > `{workspace}/.golish/transcripts` > `~/.golish/transcripts`（与 AGENTS.md §8 运行日志位置一致）。
- `EventCoordinator` 是单任务 actor，跨任务通信走 `CoordinatorHandle`，别绕过它直接共享状态。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-events
```
