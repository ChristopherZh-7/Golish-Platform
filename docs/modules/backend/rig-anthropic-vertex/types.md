# rig-anthropic-vertex / types

> **一句话职责**：Anthropic Vertex AI API 的请求/响应类型——messages（thinking/cache/system block + ContentBlock/ImageSource/Role/Message）、request（ToolDefinition/CompletionRequest）、response（Usage/StopReason/CompletionResponse）、streaming（SSE 事件）、web_tools（Claude 原生 web_search/web_fetch）。

- **类型**：目录模块（属于 crate [`rig-anthropic-vertex`](../rig-anthropic-vertex.md)）
- **路径**：`backend/crates/rig-anthropic-vertex/src/types/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 Claude 请求体/响应/消息块/SSE 事件/web tool 的 wire 类型时

## 职责

定义 Claude Vertex API 的全部 wire 类型，按 concern 拆但 flat re-export 保持 `rig_anthropic_vertex::types::*` 稳定面：`messages`（核心消息形状 + thinking/cache/system）、`request`（请求体）、`response`（含 text/tool/thinking accessor）、`streaming`（SSE 事件）、`web_tools`（server tool 结果联合类型）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `Message` / `ContentBlock` / `ImageSource` / `Role`（messages） | 核心消息类型 |
| `CompletionRequest` / `ToolDefinition`（request） | 请求体 |
| `CompletionResponse` / `Usage` / `StopReason`（response） | 响应 |
| `StreamEvent` / `ContentDelta` / `Citation`（streaming） | SSE 事件 |
| web_tools（web_search/web_fetch 结果） | server tool |

## 关键文件

| 文件 | 作用 |
|---|---|
| `messages.rs` / `request.rs` / `response.rs` | 消息 / 请求 / 响应 |
| `streaming.rs` / `web_tools.rs` | SSE 事件 / server tool |

## 依赖

- `serde`

## 注意事项 / 坑

- flat re-export 保持 `types::*` 稳定面；加类型沿用。
- 升级 `rig-core` 时这些 wire 类型与 `CompletionModel` 映射需对齐（fork 维护点）。

## 测试入口

```bash
cd backend && cargo nextest run -p rig-anthropic-vertex types
```
