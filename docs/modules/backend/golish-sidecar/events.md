# golish-sidecar / events

> **一句话职责**：sidecar 捕获的事件类型——`SessionEvent`（从 agent 交互抽取的语义信息，供存储/查询）+ `SidecarEvent`（发前端的 UI 通知）+ checkpoint / commit 边界检测 / 导出。

- **类型**：目录模块（属于 crate [`golish-sidecar`](../golish-sidecar.md)）
- **路径**：`backend/crates/golish-sidecar/src/events/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加/改 sidecar 事件类型（`SessionEvent` / `SidecarEvent` / `EventType` / `DecisionType` / `FileOperation`）时
- 改 checkpoint、commit 边界检测（`CommitBoundaryDetector`）、会话导出时

## 职责

定义 sidecar 的两类事件：`SessionEvent`（语义信息，存储/查询用）与 `SidecarEvent`（前端实时通知）。另含 checkpoint、commit 边界检测、会话导出类型。

## 公开接口

| 符号 | 说明 |
|---|---|
| `SessionEvent` | 语义会话事件（存储/查询） |
| `SidecarEvent` | 发前端的 UI 事件 |
| `EventType` / `DecisionType` / `FeedbackType` / `FileOperation` | 事件/决策/反馈/文件操作分类 |
| `CommitBoundaryDetector` / `CommitBoundaryInfo` | commit 边界检测 |
| `Checkpoint` / `SidecarSession` / `SessionExport` | 检查点 / 会话 / 导出 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `session_event.rs` / `ui_events.rs` | `SessionEvent` / `SidecarEvent` |
| `event_type.rs` | `EventType`/`DecisionType`/`FeedbackType`/`FileOperation` |
| `commit_boundary.rs` / `checkpoint.rs` / `export.rs` | 边界检测 / 检查点 / 导出 |

## 依赖

- `serde`、`chrono`；crate 内 `capture`/`processor` 消费

## 注意事项 / 坑

- 模块标 `#![allow(dead_code)]`（部分类型为未来用）。
- `SessionEvent`（存储语义）与 `SidecarEvent`（UI 通知）是**两类**，别混——前者进存储/查询，后者发前端。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-sidecar events
```
