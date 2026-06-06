# rig-zai-sdk

> **一句话职责**：rig 的 **provider fork**——为 Z.AI（GLM 系列）提供原生 Rust SDK，实现 rig-core 的 `CompletionModel`，含 SSE 流式 + tool call 累积、始终开启 thinking/reasoning、伪 XML tool call 解析。

- **类型**：crate（rig provider fork · in-tree）
- **路径**：`backend/crates/rig-zai-sdk/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 接入/调试 Z.AI GLM 模型（glm-5 / glm-4.7 / glm-4-flash …）时
- 改 SSE 流式 + tool call 累积（`streaming`）、伪 XML tool call 解析（`text_tool_parser`）时
- 改 Z.AI wire 类型（`types`）或加新 GLM 模型常量时

## 职责

为 rig 生态提供 Z.AI provider：直接按 Z.AI API 规范发 HTTP（原生 SDK，非包第三方），实现 `CompletionModel`。thinking/reasoning 始终开启。是 4 个 in-tree rig provider fork 之一。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `Client`（`new` / `completion_model` / `api_key`） | 用 API key 构造 |
| `CompletionModel` / `StreamingResponseData` / `StreamingUsage` | rig-core 实现 + 流式数据/用量 |
| `ZaiError` | 错误类型 |
| `types::{ChatCompletionChunk, CompletionRequest, Message, MessageContent, ToolCall, ToolDefinition, Usage, …}` | wire 类型 |
| `models::{GLM_5, GLM_4_7, GLM_4_6, GLM_4_FLASH, GLM_4_PLUS, GLM_4V, GLM_4_ALLTOOLS, …}` | 可用模型常量 |

## 依赖

- **内部**：`golish-json-repair`（修复畸形 LLM JSON 工具参数）
- **外部**：`rig-core`、`reqwest`（json、stream）、`futures`、`regex`（解析伪 XML tool calls）、`bytes`、`nanoid`

## 被谁依赖 / 改动影响面

仅 `golish-llm-providers`（统一 provider 抽象）。agent 栈经 llm-providers 间接使用，影响面比 anthropic/openai fork 小。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `completion/` | completion 实现（含 `StreamingResponseData`） | [→](rig-zai-sdk/completion.md) |

## 关键文件

`client.rs`、`streaming.rs`、`text_tool_parser.rs`（伪 XML tool call 解析）、`types.rs`、`error.rs`、`lib.rs`（含单测）。

## 注意事项 / 坑

- 部分 GLM 模型经**伪 XML tool call**返回，需 `text_tool_parser` + `regex` 解析（与标准 function calling 不同路径）。
- thinking/reasoning **始终开启**（见 doc 注释），不可关。
- `models::GLM_4` 实际指向 `glm-4.7`（legacy 别名），别误以为是旧 glm-4。
- 是 rig in-tree fork：升级 `rig-core` 需对齐 `CompletionModel` 签名；能力 metadata 在 `golish-models`。

## 测试入口

```bash
cd backend && cargo nextest run -p rig-zai-sdk
```
