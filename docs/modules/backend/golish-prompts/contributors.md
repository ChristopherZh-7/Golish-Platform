# golish-prompts / contributors

> **一句话职责**：prompt 贡献者——实现 `PromptContributor` trait 提供上下文感知的 system prompt 片段：provider 内置工具、skills 注入、tavily 工具。

- **类型**：目录模块（属于 crate [`golish-prompts`](../golish-prompts.md)）
- **路径**：`backend/crates/golish-prompts/src/contributors/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 加/改 system prompt 的动态片段贡献者（provider 工具 / skills / tavily）时
- 排查某段 prompt 上下文从哪来、贡献者优先级/组装时

## 职责

每个贡献者实现 `PromptContributor`，按运行上下文产出 prompt 片段。本模块含 3 个：provider 内置工具、skills、tavily 工具。

## 公开接口

| 符号 | 说明 |
|---|---|
| `ProviderBuiltinToolsContributor` | provider 内置工具片段 |
| `SkillsPromptContributor` | Agent Skills 注入片段 |
| `TavilyToolsContributor` | tavily 工具片段 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `provider_tools.rs` / `skills.rs` / `tavily_tools.rs` | 3 个贡献者实现 |

## 依赖

- `golish-core`（`PromptContributor` trait）；crate 内 registry

## 注意事项 / 坑

- **DAG 注意（A1）**：`SubAgentPromptContributor` 已**移到** `golish-sub-agents::prompt_contributor`（曾在此但引入对 sibling 域 crate 的回边）；`create_default_contributors` 组合器移到 `golish-agent-bridge::contributors`（bridge 是天然装配点）。别把这些移回来。
- 贡献者是上下文感知的：改片段注意对 chat/task 两模式的影响。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-prompts contributors
```
