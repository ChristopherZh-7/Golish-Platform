# golish-llm-providers

> **一句话职责**：统一的 LLM provider 抽象——OpenRouter / Anthropic(Vertex) / OpenAI / Ollama / Gemini / Groq / xAI / Z.AI(GLM) / DeepSeek / Xiaomi 等一套接口。

- **类型**：crate（Layer 2 基础设施）
- **路径**：`backend/crates/golish-llm-providers/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加/改 LLM provider、provider 配置、模型能力映射、OpenRouter 偏好时
- 某 provider（GLM/Xiaomi/DeepSeek/Vertex）调用异常时

## 职责

为各 LLM provider 提供统一调用接口。多数走 rig-core 内置 provider，少数走 in-tree fork（Anthropic on Vertex、Z.AI GLM、Gemini Vertex、OpenAI Responses）。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `provider_trait::*` | provider 统一 trait |
| `provider_config::*` / `openai_config::*` | provider/OpenAI 配置 |
| `model_capabilities::*` / `reasoning_models::*` | 能力/推理模型 |
| `deepseek::*` / `xiaomi::*` | 特定 provider 实现 |
| `openrouter_preferences_to_json(prefs)` | OpenRouter 偏好转 JSON |

## 依赖

- **内部**：`golish-models`、`golish-settings`、`rig-anthropic-vertex`、`rig-openai-responses`、`rig-zai-sdk`、`rig-gemini-vertex`
- **外部**：`rig-core`

## 被谁依赖 / 改动影响面

`golish`、`golish-agent-app`、`golish-agent-kit`、`golish-agent-bridge`、`golish-agent-runtime`、`golish-prompts`、`golish-sub-agents`。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `provider_trait/` | provider 统一 trait | [→](golish-llm-providers/provider_trait.md) |
| `model_capabilities/` | 模型能力映射 | [→](golish-llm-providers/model_capabilities.md) |
| `deepseek/` | DeepSeek 实现 | [→](golish-llm-providers/deepseek.md) |
| `xiaomi/` | Xiaomi MiMo 实现 | [→](golish-llm-providers/xiaomi.md) |

## 注意事项 / 坑

- 4 个 `rig-*` fork 是本 crate 的专用依赖（Vertex/GLM/Gemini/Responses）；改 provider 行为常要连带看对应 fork。
- 相关设计：`docs/design/2026-05-25-llm-models-json-driven.md`、`docs/design/2026-05-27-add-xiaomi-mimo-provider.md`、`docs/tool-use.md`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-llm-providers
```
