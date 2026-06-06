# golish-llm-providers / provider_trait

> **一句话职责**：`LlmProvider` trait + provider-impl 分派——每个 provider 一个子 impl，工厂（`create_provider` / `create_client_for_model`）按 `AiProvider` 选实现，用模型注册表（能力检测）取代字符串匹配，消除 `create_*_components` 与 `LlmClientFactory` 的重复。

- **类型**：目录模块（属于 crate [`golish-llm-providers`](../golish-llm-providers.md)）
- **路径**：`backend/crates/golish-llm-providers/src/provider_trait/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加新 LLM provider 或改某 provider 的 client 创建逻辑时
- 改 provider 选择/分派（`create_provider` / `create_client_for_model`）或凭据校验时
- reasoning vs 标准 OpenAI 等 client 变体选择问题时

## 职责

`LlmProvider` trait 封装每个 provider 的 client 创建（用 `golish-models` 注册表查能力，而非字符串匹配判断 reasoning/vision 等），工厂按 `AiProvider` 分派到具体 impl。统一了组件构建与 `LlmClientFactory` 两条路径。

## 公开接口

| 符号 | 说明 |
|---|---|
| `LlmProvider`（trait：`provider_type` / `provider_name` / `create_client` / `validate_credentials`） | provider 抽象 |
| `create_provider` / `create_client_for_model` | 按 `AiProvider` 选 impl 的工厂 |
| （各 provider impl 子模块） | 具体 provider 的 client 创建 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `LlmProvider` trait + 工厂分派 |
| （各 provider impl 文件） | 每 provider 一个 impl |

## 依赖

- `golish_models`（`AiProvider`/`ModelCapabilities`/`get_model_capabilities`）、`async_trait`、crate 内 `LlmClient`、4 个 rig fork

## 注意事项 / 坑

- **用注册表查能力，不要字符串匹配**：判断模型是否 reasoning/vision 走 `golish-models`，别 `model.starts_with(...)`（设计就是为消除这种）。
- 新增 provider：加 impl + 在工厂分派 + 在 `golish-models` 登记能力 + `AiProvider` 枚举（golish-settings）。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-llm-providers provider_trait
```
