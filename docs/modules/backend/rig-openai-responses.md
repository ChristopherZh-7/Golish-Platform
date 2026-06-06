# rig-openai-responses

> **一句话职责**：rig 的 **provider fork**——包 `async-openai` 把 OpenAI **Responses API**（而非 Chat Completions）实现为 rig-core 的 `CompletionModel`，**显式分离 reasoning 事件**，支持 o1/o3/gpt-5.x 推理模型与 reasoning effort 配置。

- **类型**：crate（rig provider fork · in-tree）
- **路径**：`backend/crates/rig-openai-responses/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 接入/调试 OpenAI reasoning 模型（o1/o3/o4/gpt-5.x）、Responses API、reasoning effort 时
- 改流式事件映射（`stream_map` / `response`），尤其 reasoning delta 与 text delta 的分离时
- 改请求构建（`request/`）或判断模型是否为 reasoning 模型（`is_reasoning_model`）时

## 职责

薄适配层：wrap `async-openai` 实现 rig-core `CompletionModel`，用 OpenAI 较新的 Responses API，并对 reasoning 模型做显式流式事件处理。是 4 个 in-tree rig provider fork 之一。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `Client`（`new`） / `ReasoningEffort`（low/medium/high） | 客户端 + 推理强度配置 |
| `CompletionModel`（`with_reasoning_effort`） | rig-core `CompletionModel` 实现 |
| `OpenAiResponsesError` | 错误类型 |
| `StreamingResponseData` | 流式响应数据 |
| `is_reasoning_model(&str) -> bool` | 判定 o1/o3/o4/gpt-5 系列 |

## 依赖

- **内部**：`golish-json-repair`（修复畸形 LLM JSON 工具参数）
- **外部**：`rig-core`、`async-openai` 0.32（features：`responses`、`rustls`）、`futures`、`nanoid`、`base64`

## 被谁依赖 / 改动影响面

`golish-llm-providers`、`golish-agent-kit`、`golish-agent-runtime`、`golish-agent-bridge`。改流式/reasoning 契约会波及整条 agent LLM 调用链。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `request/` | 请求构建（纯转换，无 HTTP） | [→](rig-openai-responses/request.md) |

## 关键文件

`client.rs`、`completion.rs`、`response.rs`、`stream_map.rs`、`error.rs`、`lib.rs`（含 `is_reasoning_model` + 单测）。

## 注意事项 / 坑

- **核心契约**：reasoning delta 映射到 `RawStreamingChoice::ReasoningDelta`，**绝不**与 text delta 混淆——改 `stream_map` 时务必保持分离。
- 用 **Responses API**（非 Chat Completions），与上游 async-openai 默认路线不同。
- 是 rig in-tree fork：升级 `rig-core` / `async-openai` 需对齐签名。
- 模型判定用前缀匹配（见单测），新增 reasoning 系列时同步 `is_reasoning_model`。

## 测试入口

```bash
cd backend && cargo nextest run -p rig-openai-responses
```
