# rig-gemini-vertex

> **一句话职责**：rig 的 **provider fork**——把 Google Gemini（部署在 Vertex AI）实现为 rig-core 的 `CompletionModel`，含 GCP 认证与流式。

- **类型**：crate（rig provider fork · in-tree）
- **路径**：`backend/crates/rig-gemini-vertex/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 接入/调试 Vertex 上的 Gemini（ADC 认证、project、region、模型 id）时
- 改 Gemini 流式（`streaming`，基于 `async-stream`）、请求/响应类型（`completion/`、`types`）时
- 加新 Gemini 模型常量时

## 职责

为 rig 生态提供「Gemini on Vertex AI」provider，实现 `CompletionModel` trait。是 4 个 in-tree rig provider fork 之一。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `Client`（`from_env` / `completion_model`） | 用 Application Default Credentials + project + region 构造 |
| `CompletionModel` | rig-core `CompletionModel` 实现 |
| `GeminiVertexError` | 错误类型 |
| `types::*` | wire 类型 |
| `models::{GEMINI_3_PRO_PREVIEW, GEMINI_3_FLASH_PREVIEW, GEMINI_2_5_PRO, GEMINI_2_5_FLASH, GEMINI_2_5_FLASH_LITE, GEMINI_2_0_FLASH, GEMINI_2_0_FLASH_LITE}` | 可用模型常量（2.0 系列 2026-03 弃用） |

## 依赖

- **内部**：无（仅 `rig-core`）
- **外部**：`rig-core`、`gcp_auth`、`reqwest`（stream）、`futures`、`async-stream`、`base64`、`nanoid`

## 被谁依赖 / 改动影响面

仅 `golish-llm-providers`（统一 provider 抽象）。比其它 rig fork 影响面小（agent 栈不直接依赖它，经 llm-providers 间接使用）。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `completion/` | `CompletionModel`（convert/model_impl） | [→](rig-gemini-vertex/completion.md) |

## 关键文件

`client.rs`、`streaming.rs`、`types.rs`、`error.rs`。

## 注意事项 / 坑

- 是 **rig 上游的 in-tree fork**：升级 `rig-core` 需对齐 `CompletionModel` 签名。
- 流式用 `async-stream`（与 anthropic-vertex 的 `stream_map` 路线不同）。
- 模型常量含 Preview / Deprecated 标记，真实可用性以 Vertex 为准；能力 metadata 在 `golish-models`。

## 测试入口

```bash
cd backend && cargo nextest run -p rig-gemini-vertex
```
