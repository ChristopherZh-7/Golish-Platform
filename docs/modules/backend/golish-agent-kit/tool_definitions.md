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
- `ToolSelectionConfig::main_agent()` 额外开放的 security-analysis 只读工具要和 execution_mode/static policy、direct executor 一起同步；例如 `check_stage_asset_coverage` / `stage_worklist_status` / `stage_worklist_next` 既要能被 active stage 看到，也不能泄漏到普通 lead turn。
- `log_operation` / `discover_apis` / `save_js_analysis` / `fingerprint_target` / `log_scan_result` 是未携带 immutable target owner witness 的 legacy mutation tools；`ToolSelectionConfig::main_agent()` 不再暴露它们。active harness 必须使用 stage-specific guarded producer 落业务事实，不能把这些旧写接口重新加回默认工具集。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-kit tool_definitions
```
