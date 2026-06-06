# rig-anthropic-vertex

> **一句话职责**：rig 的 **provider fork**——把 Anthropic Claude（部署在 Google Cloud Vertex AI）实现为 rig-core 的 `CompletionModel`，含 GCP 认证、流式、server tools（web search/fetch）配置。

- **类型**：crate（rig provider fork · in-tree）
- **路径**：`backend/crates/rig-anthropic-vertex/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 接入/调试 Vertex 上的 Claude（service account 认证、region、模型 id）时
- 改 Claude 流式事件映射（`stream_map` / `streaming/`）、请求/响应类型（`request`/`response`/`types`）时
- 加新 Claude 模型常量或 server tools（web search/fetch）配置时

## 职责

为 rig 生态提供「Claude on Vertex AI」provider，实现 `CompletionModel` trait。是 4 个 in-tree rig provider fork 之一（AGENTS.md §0）。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `Client`（`from_service_account` / `completion_model`） | 从 service account JSON + project + region 构造 |
| `CompletionModel` | rig-core `CompletionModel` 实现 |
| `ServerToolsConfig` / `WebFetchConfig` / `WebSearchConfig` | Anthropic server tools 配置 |
| `AnthropicVertexError` | 错误类型 |
| `StreamingCompletionResponseData` | 流式响应数据 |
| `types::*` | wire 类型 |
| `models::{CLAUDE_OPUS_4_6, CLAUDE_SONNET_4_6, CLAUDE_OPUS_4_5, CLAUDE_SONNET_4_5, CLAUDE_HAIKU_4_5}` | 可用模型常量 |

## 依赖

- **内部**：无（仅 `rig-core`）
- **外部**：`rig-core`、`gcp_auth`（GCP 认证）、`reqwest`（stream）、`futures`、`bytes`、`base64`、`nanoid`

## 被谁依赖 / 改动影响面

`golish-llm-providers`（统一 provider 抽象）、`golish-agent-kit`、`golish-agent-runtime`、`golish-agent-bridge`。改 `CompletionModel`/流式契约会波及整条 LLM 调用链。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `streaming/` | SSE 流式响应处理 | [→](rig-anthropic-vertex/streaming.md) |
| `types/` | wire 类型定义 | [→](rig-anthropic-vertex/types.md) |

## 关键文件

`client.rs`、`completion.rs`、`config.rs`、`request.rs`、`response.rs`、`stream_map.rs`、`error.rs`。

## 注意事项 / 坑

- 是 **rig 上游的 in-tree fork**：升级 `rig-core` 时需对齐 fork 的 `CompletionModel` 签名。
- 走 Vertex（非 Anthropic 直连），认证用 `gcp_auth`（service account / ADC）；模型 id 带 `@default` / 日期后缀。
- 模型常量是清单，真实可用性以 Vertex region/配额为准；不要把常量当能力声明（能力 metadata 在 `golish-models`）。

## 测试入口

```bash
cd backend && cargo nextest run -p rig-anthropic-vertex
```
