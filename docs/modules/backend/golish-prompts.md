# golish-prompts

> **一句话职责**：Golish AI agent 的 prompt 组装系统——可插拔 `PromptContributor` + 按优先级组装的 registry + 顶层 system prompt 构建 + codex 变体 + 摘要器。

- **类型**：crate（Layer 2.5 基础设施）
- **路径**：`backend/crates/golish-prompts/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 system prompt、prompt 贡献者、prompt 组装顺序、上下文摘要/压缩 prompt 时
- agent 模式指令、团队委派指令相关时

## 职责

owns 跨切面的 prompt 基础设施：贡献者机制、按优先级组装、system prompt 构建、codex 风格变体、LLM 驱动的会话摘要。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `PromptContributorRegistry` | 按优先级组装贡献者 |
| `ProviderBuiltinToolsContributor` / `SkillsPromptContributor` / `TavilyToolsContributor` | 内置贡献者 |
| `build_codex_style_prompt` | codex 风格 prompt |
| `generate_summary` / `build_summarizer_user_prompt` / `SUMMARIZER_SYSTEM_PROMPT` | 摘要器 |

## 依赖

- **内部**：`golish-core`、`golish-llm-providers`
- **外部**：`rig-core`

## 被谁依赖 / 改动影响面

`golish`、`golish-agent-app`、`golish-agent-kit`、`golish-agent-bridge`、`golish-agent-runtime`。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `contributors/` | 各类 PromptContributor 实现 | [→](golish-prompts/contributors.md) |
| `system_prompt/` | 顶层 system prompt 构建 | [→](golish-prompts/system_prompt.md) |

## 关键文件

`prompt_registry.rs`、`codex_prompt.rs`、`summarizer.rs`。

## 注意事项 / 坑

- **分层铁律**：本 crate **不依赖** golish-sub-agents（`SubAgentPromptContributor` 已搬到 sub-agents，`create_default_contributors` 在 agent-bridge），别再引回这条 back-edge。
- 相关：`docs/prompt-contributions.md`、`docs/superpowers/plans/prompt-generation-ui-plan.md`、`docs/planning-system.md`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-prompts
```
