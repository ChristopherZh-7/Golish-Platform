# golish-context

> **一句话职责**：LLM 上下文窗口与 token 预算管理——token 计数、上下文压缩（compaction）、预算告警、工具输出截断。

- **类型**：crate（Layer 2/3）
- **路径**：`backend/crates/golish-context/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 token 计数、上下文压缩/裁剪、token 预算告警、工具输出截断时
- 出现「上下文爆窗 / 工具输出太长 / 压缩没触发」时

## 职责

管理 LLM 上下文窗口：统计 token、按预算压缩历史、对超长工具输出做截断聚合。是 agent 长任务不爆窗的关键。

## 公开接口 / 关键类型

| 符号 | 说明 |
|---|---|
| `ContextManager` / `ContextManagerConfig` / `ContextTrimConfig` | 上下文管理与压缩 |
| `CompactionCheck` / `CompactionState` / `ContextSummary` | 压缩判定与状态 |
| `TokenBudgetManager` / `TokenBudgetConfig` / `TokenUsageStats` / `TokenAlertLevel` | token 预算 |
| `truncate_by_tokens` / `truncate_by_chars` / `aggregate_tool_output` | 截断工具 |
| `DEFAULT_MAX_CONTEXT_TOKENS` / `MAX_TOOL_RESPONSE_TOKENS` / `BYTE_FUSE_LIMIT` | 关键常量 |

## 依赖

- **内部**：无（仅外部 `rig-core`）

## 被谁依赖 / 改动影响面

`golish-agent-kit`、`golish-agent-runtime`、`golish-agent-bridge`、`golish`、`golish-agent-app`。改截断/压缩阈值影响所有长会话。

## 子模块（目录模块，各有卡片）

| 子模块 | 一句话 | 卡片 |
|---|---|---|
| `context_manager/` | 上下文压缩/裁剪/告警 | [→](golish-context/context_manager.md) |
| `token_budget/` | token 预算与用量统计 | [→](golish-context/token_budget.md) |

## 关键文件

| 文件 | 作用 |
|---|---|
| `token_trunc.rs` | 按 token/字符截断、聚合工具输出 |

## 注意事项 / 坑

- 工具输出有 token 上限（`MAX_TOOL_RESPONSE_TOKENS`），超出会截断——排查「工具结果被砍」看这里。
- 压缩是有状态的（`CompactionState`），改压缩逻辑注意幂等与重复触发。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-context
```
