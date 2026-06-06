# golish-llm-providers / model_capabilities

> **一句话职责**：模型能力检测——vision / thinking-history / temperature / web search 支持判定，以及 per-provider 流式 quirks（reasoning 处理、thinking 禁用字段）。

- **类型**：目录模块（属于 crate [`golish-llm-providers`](../golish-llm-providers.md)）
- **路径**：`backend/crates/golish-llm-providers/src/model_capabilities/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改模型能力判定（是否支持 vision/temperature/web search/thinking history）时
- 改 per-provider 流式 quirks（`ProviderStreamQuirks` / `ReasoningHandling` / `ThinkingDisableField`）时

## 职责

提供模型能力检测与 provider 流式 quirks：`ModelCapabilities` 聚合能力位，helpers 判 temperature/web search，`quirks` 解析每 provider 的流式差异（reasoning 怎么处理、thinking 禁用字段、模型 override）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `ModelCapabilities` | 模型能力聚合 |
| `VisionCapabilities` | vision 能力 |
| `model_supports_temperature` / `openai_supports_web_search` | 能力判定 helper |
| `resolve_stream_quirks` / `ProviderStreamQuirks` / `ReasoningHandling` / `ThinkingDisableField` / `ModelOverride` | 流式 quirks 解析 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `capabilities.rs` | `ModelCapabilities` |
| `vision.rs` | `VisionCapabilities` |
| `helpers.rs` | temperature / web search 判定 |
| `quirks.rs` | per-provider 流式 quirks |

## 依赖

- crate 内 + `golish-models`（能力来源）

## 注意事项 / 坑

- 能力应源自 `golish-models` 注册表 metadata（JSON 驱动），别在此硬编码字符串匹配。
- `quirks` 处理各家 reasoning/thinking 的协议差异——加新 provider 的流式特性在此登记。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-llm-providers model_capabilities
```
