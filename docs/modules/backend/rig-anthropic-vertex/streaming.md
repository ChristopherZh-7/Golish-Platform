# rig-anthropic-vertex / streaming

> **一句话职责**：Anthropic Vertex AI 的 SSE 流式处理——`StreamingResponse` + `StreamChunk`：parse（SSE 行解析）/ poll（`Stream` impl 驱动字节流）/ event（事件→chunk，处理 thinking 签名累积、tool-use 起始、web search/fetch 结果、最终 usage 汇总）。

- **类型**：目录模块（属于 crate [`rig-anthropic-vertex`](../rig-anthropic-vertex.md)）
- **路径**：`backend/crates/rig-anthropic-vertex/src/streaming/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 Claude 流式 SSE 解析、`StreamChunk` 映射、thinking 签名累积、web tool 结果、usage 汇总时

## 职责

按 SSE 生命周期分阶段处理 Claude Vertex 流：`mod.rs` 公开 `StreamingResponse` + `StreamChunk`；`parse` SSE 行→`StreamEvent`；`poll` 实现 `Stream` 驱动字节泵；`event` 把事件译成 `StreamChunk`（thinking 签名累积、tool-use 起始、web search/fetch、最终 usage）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `StreamingResponse` | 流式响应（`Stream` impl） |
| `StreamChunk` | 流式块枚举 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `StreamingResponse` + `StreamChunk` |
| `parse.rs` / `poll.rs` / `event.rs` | SSE 解析 / Stream 泵 / 事件→chunk |

## 依赖

- crate 内 `types`（`StreamEvent`/`Usage`）、`futures::Stream`

## 注意事项 / 坑

- thinking 签名需**累积**（跨多个 SSE 事件拼）；别按单事件处理。
- reasoning/text/tool 块要分清（与上层 rig `RawStreamingChoice` 映射一致）。

## 测试入口

```bash
cd backend && cargo nextest run -p rig-anthropic-vertex streaming
```
