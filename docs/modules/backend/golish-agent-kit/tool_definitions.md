# golish-agent-kit / tool_definitions

> **一句话职责**：工具定义/选择——`ToolPreset`（Minimal/Standard/Full）+ `ToolSelectionConfig`（preset + add/disable override）+ schema sanitize（OpenAI strict / Anthropic 兼容）+ 手写描述符（run_command/ask_human/sub_agent_* shim）+ `get_*_tool_definitions*` 组合。

- **类型**：目录模块（属于 crate [`golish-agent-kit`](../golish-agent-kit.md)）
- **路径**：`backend/crates/golish-agent-kit/src/tool_definitions/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 LLM 看到的工具集（preset/allow/block）、工具 schema sanitize、描述 override 时
- 改 `run_command`/`ask_human`/`sub_agent_*` 的手写描述符时

## 职责

决定 LLM 看到哪些工具及其 schema。`preset`（preset 枚举 + allow-list）；`config`（`ToolSelectionConfig` + `main_agent` factory）；`sanitize`（递归 JSON Schema 变换，适配 OpenAI strict + Anthropic）；`definitions`（手写描述符）；`selection`（`get_*_tool_definitions*` 组合 preset 过滤 + sanitize + 描述 override + `filter_tools_by_allowed`）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `ToolPreset`（Minimal/Standard/Full） | 工具预设 |
| `ToolSelectionConfig`（`main_agent`） | preset + add/disable override |
| `get_all_tool_definitions_with_config` / `get_tool_definitions_for_preset` / `get_tool_definitions_with_config` | 组合入口 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `preset.rs` / `config.rs` | 预设 + 选择配置 |
| `sanitize.rs` | JSON Schema sanitize（strict/Anthropic 兼容） |
| `definitions.rs` / `selection.rs` | 手写描述符 / 组合选择 |

## 依赖

- crate 内 `tool_execution`；`golish-tools`（registry schema）、`serde_json`

## 注意事项 / 坑

- schema sanitize 适配 provider 差异（OpenAI strict mode）；改 schema 形状要跑两家。
- 工具 schema 与 `golish-tools` 实现要一致；改参数两边同步（这是 LLM「能看到哪些工具」的出口）。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-kit tool_definitions
```
