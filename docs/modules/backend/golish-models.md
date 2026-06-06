# golish-models

> **一句话职责**：LLM 模型注册表与能力定义——用显式 metadata（能力、温度、工具调用支持等）取代字符串匹配启发式。

- **类型**：crate（Layer 2 基础设施）
- **路径**：`backend/crates/golish-models/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加/改模型定义、模型能力（是否支持温度/工具调用）、某 provider 的模型列表时
- 出现「模型不识别 / 能力判断错」时

## 职责

集中模型定义与能力的注册表。模型来源两类：static（预定义 `MODEL_REGISTRY`）+ dynamic（运行时发现，如 Ollama `/api/tags`）。用 metadata 显式表达能力，避免按模型名字符串猜。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `get_model(id)` / `get_models_for_provider(provider)` | 查模型 |
| `ModelCapabilities` / `ModelDescriptor` / `CapabilitiesDescriptor` | 模型与能力描述 |
| `MODEL_REGISTRY`（via `registry::*`） | 静态注册表 |
| `load_provider_models` / `merge_capabilities` / `embedded_defaults_for` | provider 模型加载/合并 |
| `AiProvider`（re-export 自 golish-settings） | provider 枚举 |

## 依赖

- **内部**：`golish-settings`（取 `AiProvider`）

## 被谁依赖 / 改动影响面

`golish-llm-providers`、`golish`、`golish-agent-app`。改能力字段会影响 LLM 调用参数（温度/工具）。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `descriptors/` | 模型/能力描述符与 provider 模型文件加载 | [→](golish-models/descriptors.md) |
| `providers/` | 各 provider 模型定义 | [→](golish-models/providers.md) |

## 关键文件

| 文件 | 作用 |
|---|---|
| `capabilities.rs` | 能力类型 |
| `registry.rs` | 静态模型注册表 |
| `tool_use_profile.rs` | 工具调用能力画像 |

## 注意事项 / 坑

- 模型相关参考 `docs/design/2026-05-25-llm-models-json-driven.md`（JSON 驱动模型）。
- 加新 provider/模型时同步 static 注册表 + 对应 `providers/`，能力用 metadata 表达而非字符串匹配。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-models
```
