# golish-context / token_budget

> **一句话职责**：token 预算跟踪——用量统计、per-model 上下文上限、告警阈值，以及运行时 `TokenBudgetManager`。

- **类型**：目录模块（属于 crate [`golish-context`](../golish-context.md)）
- **路径**：`backend/crates/golish-context/src/token_budget/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 token 用量统计、(provider,model) 预算汇总、告警等级阈值时
- 改 per-model 上下文上限（`ModelContextLimits`）或工具响应 token 上限常量时

## 职责

跟踪 token 预算：`TokenUsage` 用量 DTO、`ModelContextLimits`（每模型最大上下文 token）、`TokenUsageStats`（按 provider,model 汇总）+ `TokenAlertLevel` 告警，运行时由 `TokenBudgetManager` 驱动。

## 公开接口

| 符号 | 说明 |
|---|---|
| `TokenBudgetManager` | 预算跟踪运行时 |
| `TokenBudgetConfig` | 预算配置 |
| `ModelContextLimits` | per-model 最大上下文 token |
| `TokenUsage` / `TokenUsageStats` / `TokenAlertLevel` | 用量 DTO / 汇总统计 / 告警等级 |
| `DEFAULT_MAX_CONTEXT_TOKENS` / `MAX_TOOL_RESPONSE_TOKENS` | 默认上下文上限 / 工具响应 token 上限 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `manager.rs` | `TokenBudgetManager` 运行时 |
| `limits.rs` | per-model 上下文上限 |
| `stats.rs` | 用量统计 + 告警等级 |
| `usage.rs` | `TokenUsage` DTO + 常量 |
| `config.rs` | 预算配置 + 工具/上下文常量 |

## 依赖

- crate 内基础；被 `context_manager/`（压缩决策）消费

## 注意事项 / 坑

- `MAX_TOOL_RESPONSE_TOKENS` 决定工具输出何时被截断——改它会影响 agent 看到的工具结果完整度。
- 预算与压缩（`context_manager/`）配对：阈值改动要两边对齐。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-context token_budget
```
