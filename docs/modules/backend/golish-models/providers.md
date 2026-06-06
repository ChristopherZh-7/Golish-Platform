# golish-models / providers

> **一句话职责**：per-provider 模型目录 + provider 元数据——13 家 provider（Vertex/Anthropic/OpenAI/Gemini/Groq/xAI/Z.AI/Ollama/OpenRouter/NVIDIA/DeepSeek/Xiaomi/VertexGemini）各自的模型清单函数 + `ProviderInfo`（名称/图标/描述）。

- **类型**：目录模块（属于 crate [`golish-models`](../golish-models.md)）
- **路径**：`backend/crates/golish-models/src/providers/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加新 LLM provider 或给某 provider 加/改模型清单时
- 改 provider 元数据（前端 Settings 显示的名称/图标/描述）时

## 职责

每个 provider 一个子文件，导出 `<provider>_models()` 返回该家模型目录；`mod.rs` 聚合并提供 `get_provider_info` / `get_all_provider_info`（前端 Settings 用的 provider 元数据：name/icon/description）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `anthropic_models` / `openai_models` / `gemini_models` / `groq_models` / `xai_models` / `zai_sdk_models` / `ollama_default_models` / `openrouter_models` / `nvidia_models` / `deepseek_models` / `xiaomi_models` / `vertex_ai_models` / `vertex_gemini_models` | 各家模型清单 |
| `ProviderInfo` | provider 元数据（provider/name/icon/description） |
| `get_provider_info(AiProvider)` / `get_all_provider_info()` | 单个 / 全部 provider 元数据 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `ProviderInfo` + `get_provider_info`/`get_all_provider_info` |
| `anthropic.rs` / `openai.rs` / `gemini.rs` / `groq.rs` / `xai.rs` / `zai_sdk.rs` / `ollama.rs` / `openrouter.rs` / `nvidia.rs` / `deepseek.rs` / `xiaomi.rs` / `vertex_ai.rs` / `vertex_gemini.rs` | 各家模型目录 |

## 依赖

- `golish_settings::schema::AiProvider`、`serde`

## 注意事项 / 坑

- 模型清单与 JSON 驱动的 `descriptors/`（`resources/llm-models/*.json`）并存——确认哪条路径是某 provider 的权威来源，别两处冲突。
- `get_all_provider_info` 顺序即前端展示顺序；新增 provider 要同时改 `AiProvider` 枚举（golish-settings）+ 加 `<provider>.rs` + 加进两个 get 函数。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-models providers
```
