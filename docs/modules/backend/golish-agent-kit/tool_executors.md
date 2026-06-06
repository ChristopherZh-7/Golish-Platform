# golish-agent-kit / tool_executors

> **一句话职责**：各类工具的具体执行实现——web fetch、plan、ask_human（barrier）、memory（搜索/存储/列举 + code/guide store）、knowledge_base（wiki 漏洞知识）、security（finding 管理/分析）、graph（实体/关系知识图）、sploitus、shell helper。

- **类型**：目录模块（属于 crate [`golish-agent-kit`](../golish-agent-kit.md)）
- **路径**：`backend/crates/golish-agent-kit/src/tool_executors/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改某类工具的具体执行（memory/knowledge_base/security/graph/sploitus/ask_human/web/plan/shell）时
- 加新工具执行器或改其结果契约时

## 职责

提供 `tool_execution` 路由后落到的具体执行逻辑，按域分文件。注意 workflow 工具执行在 golish crate（避免与 WorkflowState/BridgeLlmExecutor 的循环依赖）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `execute_ask_human_tool` | ask_human barrier 工具 |
| `error_result` / `extract_string_param` / `ToolResult` | 公共 helper（common） |
| `graph` / `graph_trait` | 知识图执行 + trait |
| `knowledge_base` / `security` | wiki 知识库 / finding 管理（pub 模块） |
| memory / plan / shell / web（内部模块） | 各域执行 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `memory.rs` / `knowledge_base/` / `security.rs` | 记忆 / 知识库 / 安全分析 |
| `graph.rs` / `graph_trait.rs` | 知识图 + trait |
| `ask_human.rs` / `plan.rs` / `shell.rs` / `web.rs` / `common.rs` | barrier / 计划 / shell / web / 公共 |

## 依赖

- crate 内 `tool_execution`；`golish-tools`、`golish-pentest`（evidence/finding）、`golish-graphiti`（图，经 trait）

## 注意事项 / 坑

- workflow 工具**不在这里**（在 golish，避循环依赖）；别往这加 workflow 执行。
- graph 走 `graph_trait`（注入），不直接依赖 golish-graphiti 具体实现。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-kit tool_executors
```
