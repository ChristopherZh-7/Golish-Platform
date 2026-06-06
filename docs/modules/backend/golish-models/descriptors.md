# golish-models / descriptors

> **一句话职责**：JSON 驱动的模型注册表反序列化类型——把 `resources/llm-models/<provider>.json` 映射成运行时 `ModelDefinition`，支持 `capabilities.base` 引用 + 字段 override。

- **类型**：目录模块（属于 crate [`golish-models`](../golish-models.md)）
- **路径**：`backend/crates/golish-models/src/descriptors/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 `resources/llm-models/<provider>.json` 的 schema（新增 capability 字段、aliases、thinking_quirks）时
- 改 capabilities base 解析/合并（`capabilities.base` 引用 → 具体能力）时
- 改 JSON → `ModelDefinition` 的加载（embedded 默认 vs 文件）时

## 职责

owns 把 provider JSON 文件反序列化成模型描述符的类型（`ProviderModelsFile` / `ModelDescriptor` / `CapabilitiesDescriptor`），并解析 `capabilities.base` 引用 + override 合并。设计见 `docs/design/2026-05-25-llm-models-json-driven.md`。

## 公开接口

| 符号 | 说明 |
|---|---|
| `ProviderModelsFile` | `<provider>.json` 顶层结构（provider + default_capabilities_base + models） |
| `ModelDescriptor` | 单模型条目（id / display_name / capabilities / aliases / thinking_quirks） |
| `CapabilitiesDescriptor` | 能力 override（`None` 字段回退 base） |
| `merge_capabilities` / `resolve_capabilities_base` | base 解析 + override 合并 |
| `embedded_defaults_for` / `load_provider_models` | 内嵌默认 / 从文件加载 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | 反序列化类型（`ProviderModelsFile` 等）+ 单测 |
| `capabilities_base.rs` | base 解析 + 能力合并 |
| `loader.rs` | embedded 默认 + provider 文件加载 |

## 依赖

- `serde`/`serde_json`；resources 下的 `llm-models/*.json`

## 注意事项 / 坑

- `id` 缺失会 reject（见单测 `rejects_missing_id`）；`CapabilitiesDescriptor` 默认全 `None`（回退 base）。
- provider slug 必须匹配 `AiProvider` 变体；新增 provider 要同步 `enums::AiProvider`（在 golish-settings）+ `providers/`。
- `thinking_quirks` 目前仅 informational。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-models descriptors
```
