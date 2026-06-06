# golish-json-repair

> **一句话职责**：修复 LLM 产出的非法 JSON 工具参数，并保证工具参数始终是 JSON **object**（专治 GLM/MiMo 等 provider 的畸形输出）。

- **类型**：crate（Layer 2 基础设施，叶子工具 crate）
- **路径**：`backend/crates/golish-json-repair/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- LLM 工具参数解析失败、需要 JSON 修复时
- MiMo/GLM 等把标量当 `function.arguments` 导致下一轮 Jinja 模板崩（HTTP 500）时

## 职责

LLM（尤其 GLM）有时产出无法解析的 JSON。本 crate 先尝试标准解析，失败则用 `llm_json` 修复；并把非 object 的参数强制折成 `{}`，避免 MiMo 这类 provider 在历史回放时 `arguments.items()` 崩溃。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `parse_tool_args(s) -> Value` | 标准解析失败则修复，再失败返回 `{}` |
| `parse_tool_args_object(s) -> Value` | 保证返回 object（非 object → `{}`） |
| `ensure_tool_args_object(v) -> Value` | 把已解析值强制成 object |

## 依赖

- **内部**：无（叶子）；外部 `llm_json`、`serde_json`

## 被谁依赖 / 改动影响面

`golish-agent-kit`、`golish-agent-runtime`、`golish-sub-agents`、`rig-openai-responses`、`rig-zai-sdk`。是工具调用参数解析的统一入口。

## 关键文件

| 文件 | 作用 |
|---|---|
| `lib.rs` | 全部修复逻辑（无目录子模块） |

## 注意事项 / 坑

- `parse_tool_args_object` 把任何非 object（标量/数组）折成 `{}`，**这是有意为之**——防 Xiaomi MiMo 把裸标量（如 `example.com`）当参数导致下一轮 provider Jinja 模板 HTTP 500。改这里别放宽这个约束。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-json-repair
```
