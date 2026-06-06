# golish-settings / schema

> **一句话职责**：设置 schema 定义——`GolishSettings` 根结构（聚合 ai/api_keys/tools/ui/terminal/agent/mcp/trust/privacy/advanced/sidecar/indexer/context/telemetry/network/notifications/codebases）+ `SCHEMA_VERSION` + 全量 `Default`，全部 `#[serde(default)]` 支持部分配置。

- **类型**：目录模块（属于 crate [`golish-settings`](../golish-settings.md)）
- **路径**：`backend/crates/golish-settings/src/schema/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加/改任何设置字段（新 provider 设置、新 UI 选项、新 sidecar 配置等）时
- 改 `AiProvider` / `Theme` / `LogLevel` 等枚举、或某设置的默认值时
- bump `SCHEMA_VERSION` + 配套写 `loader::migrate_settings` 迁移时

## 职责

定义全部设置的类型与默认值。`mod.rs` 持有 `GolishSettings` 根结构（聚合各子模块顶层设置）+ master `Default` impl + `SCHEMA_VERSION`。所有 struct 用 `#[serde(default)]` 允许部分配置文件，缺字段填默认。

## 公开接口

| 符号 | 说明 |
|---|---|
| `GolishSettings` | 根设置结构（聚合所有域）+ `Default` |
| `SCHEMA_VERSION` | 当前 schema 版本（改 shape 必须 bump + 加迁移） |
| `enums::*` | `AiProvider` / `Theme` / `LogLevel` / `IndexLocation` … |
| `ai::*` | `AiSettings` / `ApiKeysSettings` / `NetworkSettings` / `SubAgentModelConfig` |
| `llm::*` | per-provider LLM 设置（`VertexAiSettings` / `OpenAiSettings` …） |
| `ui::*` | `UiSettings` / `WindowSettings` / `TerminalSettings` / `AgentSettings` / `ToolsSettings` |
| `mcp_trust::*` / `runtime::*` / `sidecar::*` | MCP/信任/隐私 · indexer/context/telemetry/langfuse · sidecar/synthesis |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `GolishSettings` 根 + `SCHEMA_VERSION` + Default |
| `enums.rs` / `defaults.rs` | 枚举 · `#[serde(default)]` 用的纯默认函数 |
| `ai.rs` / `llm/` / `ui.rs` / `mcp_trust.rs` / `runtime.rs` / `sidecar.rs` | 各域设置 struct |

## 依赖

- `serde`；被 `loader/` 与全仓库消费

## 注意事项 / 坑

- 改 shape **必须** bump `SCHEMA_VERSION` 并在 `loader::migrate_settings` 加迁移条目，否则旧 `settings.toml` 加载行为不可控。
- `schema_version` 有 `#[serde(alias = "version")]` 兼容旧字段名。
- 全字段 `#[serde(default)]` 是有意为之（前向兼容部分配置）；新增字段务必带默认。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-settings schema
```
