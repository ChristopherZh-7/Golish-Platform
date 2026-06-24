# golish-core / events

> **一句话职责**：`AgentBridge`→前端的 **wire-format 事件契约**——`AiEvent` 大枚举（故意单一巨枚举，因为它就是与前端的线协议）+ `AiEventEnvelope`（seq+ts 可靠投递包装）+ `ToolSource` + `HarnessTraceKind`。

- **类型**：目录模块（属于 crate [`golish-core`](../golish-core.md)）
- **路径**：`backend/crates/golish-core/src/events/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加/改任何 `AiEvent` 变体（新事件类型、字段）时——这是**端到端 wire 契约**，前端 ts-rs 绑定要同步
- 改事件投递包装（seq/timestamp）、tool 来源标记、harness trace 事件时
- 前端收不到某事件 / 事件字段对不上时

## 职责

定义从后端流式推送到前端的全部 AI 事件。`AiEvent` 是一个**有意为之的大枚举**（lifecycle / streaming / tools / sub-agent / context / loop / workflow / HITL / task / harness 全在内），因为它是单一 wire 契约。`AiEventEnvelope` 加 seq+ts 保证可靠有序投递。

## 公开接口

| 符号 | 说明 |
|---|---|
| `AiEvent`（+ `event_type()`） | 全部 AI 事件的大枚举（wire 契约） |
| `AiEventEnvelope` | seq + ts 包装，保证可靠投递 |
| `ToolSource` | 一次工具调用的来源（main/sub-agent/server tool 等） |
| `HarnessTraceKind` / `build_agent_path` | harness 观测事件类型 + agent_path 构造 |

均经 crate 根 `pub use events::*` 暴露为 `golish_core::events::*`。

## 关键文件

| 文件 | 作用 |
|---|---|
| `event.rs` | `AiEvent` 枚举本体（最大文件，wire 契约） |
| `envelope.rs` | `AiEventEnvelope`（seq+ts 包装） |
| `tool_source.rs` | `ToolSource` |
| `harness_trace.rs` | `HarnessTraceKind` + `build_agent_path` |
| `event_dispatch.rs` | 事件分发辅助 |

## 依赖

- crate 内 `golish_core` 基础类型；`serde`（wire 序列化）

## 注意事项 / 坑

- **不变量 I5**：`AiEvent` 跨 IPC，必须 ts-rs 同步前端；新增/改变体要同步前端绑定与所有消费方（`golish-cli-output/cli_json`、`op_trace`、前端 hooks）。
- `event.rs` 是**单一大枚举**，别拆散——拆了会破坏 wire 契约一致性。
- `ToolSource` 被 agent tool context 用作可比较字段，保持 `PartialEq + Eq` derive；否则 `AgentToolContext` 的任务本地 attribution 测试会在 `golish-core` 编译期失败。
- `should_transcript`（在 `golish-events/transcript`）会按变体过滤；加流式/sub-agent 内部事件时注意是否该落 transcript。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-core events
```
