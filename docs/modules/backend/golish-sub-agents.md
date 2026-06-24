# golish-sub-agents

> **一句话职责**：sub-agent 系统——sub-agent 定义（自定义 system prompt + 工具限制）、registry、发现/加载（YAML frontmatter）、执行器（含 udiff 应用）、prompt registry/contributor，以及默认 sub-agent 集。

- **类型**：crate（Layer 2 · agent 基础设施）
- **路径**：`backend/crates/golish-sub-agents/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 sub-agent 定义/registry、`execute_sub_agent` 执行链、嵌套深度（`MAX_AGENT_DEPTH`）时
- 改 agent 文件加载（YAML frontmatter + 文件系统发现）、默认 sub-agent 集时
- 改 sub-agent prompt 模板（tera）/ contributor / skills 注入时

## 职责

提供 sub-agent 编排基础设施：定义专门化 sub-agent、管理可用 agent 注册表、在 agent 间传递 context、带工具支持地执行 sub-agent。通过 `ToolProvider` trait 注入工具定义/执行，避免对上层 agent runtime 的反向依赖。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `SubAgentDefinition` / `SubAgentRegistry` | sub-agent 定义与注册表 |
| `SubAgentContext` / `SubAgentResult` / `AgentSource` / `MAX_AGENT_DEPTH` | 上下文/结果/来源/深度上限 |
| `execute_sub_agent` / `SubAgentExecutorContext` / `ToolProvider` | 主执行函数 + 工具注入 trait |
| `create_default_sub_agents` | 默认 sub-agent 集 |
| `discover_agents` / `AgentFileInfo` | 文件系统发现 + 加载 |
| `PromptRegistry` / `PromptContext` / `SubAgentPromptContributor` | prompt 注册/上下文/贡献者 |
| `StageToolGuard` / `StageToolHider` / `SubAgentToolRouter` / `SubAgentToolResultHook` / `PostShellHook` / `SubAgentChainPersistence` | 阶段工具守卫/路由/工具结果后处理/持久化（executor_types） |

## 依赖

- **内部**：`golish-core`、`golish-udiff`、`golish-tools`、`golish-shell-exec`、`golish-llm-providers`、`golish-json-repair`、`golish-skills`
- **外部**：`rig-core`、`serde_yaml`、`tera`、`dirs`

## 被谁依赖 / 改动影响面

`golish-agent-kit`、`golish-agent-runtime`、`golish-agent-bridge`、`golish-agent-app`、`golish`。整条 agent 栈都依赖它。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `definition/` | 定义/registry/context/result | [→](golish-sub-agents/definition.md) |
| `executor/` | 执行链（execute_sub_agent） | [→](golish-sub-agents/executor.md) |
| `executor_helpers/` | 执行辅助（content/history/helper） | [→](golish-sub-agents/executor_helpers.md) |
| `defaults/` | 默认 sub-agent 集 + prompt fallback | [→](golish-sub-agents/defaults.md) |

## 关键文件

`discovery.rs`、`file_loader.rs`、`prompt_registry.rs`、`prompt_contributor.rs`、`schemas.rs`、`transcript.rs`、`executor_types.rs`、`executor_udiff.rs`。

## 注意事项 / 坑

- `MAX_AGENT_DEPTH` 限制嵌套递归——改 sub-agent 调 sub-agent 时务必尊重深度上限，防失控。
- 工具走 `ToolProvider` trait 注入（非直接依赖上层 runtime），保持本 crate 处于 L2，不要引入向上依赖。
- 默认 `recon` 子 agent 是 `target_intel` 的 provider-only 生产者：不暴露 `list_in_scope_targets` / `pentest_run`，避免在 intel 阶段查询尚未生产的目标或 fallback 到 subfinder/dig 类扫描路径；`prober` / `enumerator` 才消费 `list_in_scope_targets`。
- `SubAgentToolResultHook` 只提供通用结果后处理注入点；具体 harness/evidence/source_query 副作用由上层 runtime 注入，避免本 crate 反向依赖 DB/harness。
- doc 注释提到的 `golish-web` / `vtcode-core` 为历史描述；当前 Cargo.toml 实际内部依赖以本卡「依赖」段为准。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-sub-agents
```
