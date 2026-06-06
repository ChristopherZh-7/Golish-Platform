# golish-agent-kit / tool_execution

> **一句话职责**：统一工具路由——`route_tool_execution` 按工具名前缀分类分派（web_fetch / web_search* / update_plan / sub_agent_* / run_command / 其余 registry 工具），消除主 loop 与 sub-agent 执行的重复。

- **类型**：目录模块（属于 crate [`golish-agent-kit`](../golish-agent-kit.md)）
- **路径**：`backend/crates/golish-agent-kit/src/tool_execution/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改工具路由分类、`route_tool_execution` 分派、HITL gating 旋钮时
- 加新工具类别或改 `run_pty_cmd` 参数归一时

## 职责

统一所有 agent 实现的工具路由。`hitl` 配置/上下文类型（`ToolExecutionConfig`/`ToolSource`/`ToolExecutionContext`，含 require_hitl/allow_sub_agents）；`direct` 执行 helper（`execute_registry_tool`/`normalize_run_pty_cmd_args` + web_fetch/update_plan/sub_agent_* 占位）；`route` 分派器（`ToolRoutingCategory` + `route_tool_execution`）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `route_tool_execution` / `ToolRoutingCategory` | 路由分派器 |
| `ToolExecutionConfig` / `ToolExecutionContext` / `ToolSource` | 配置/上下文/来源 |
| `ToolExecutionResult` / `ToolExecutionError` | 执行结果/错误 |
| `normalize_run_pty_cmd_args` | `run_pty_cmd` 参数归一 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `route.rs` | 分类 + `route_tool_execution` |
| `direct.rs` | 执行 helper + 占位处理器 |
| `hitl.rs` | 配置/上下文/HITL gating 旋钮 |

## 依赖

- crate 内 `tool_executors`（具体执行）、`tool_definitions`；`golish-tools`

## 注意事项 / 坑

- 路由按**前缀**：`run_command` 是 `run_pty_cmd` 别名；`sub_agent_*` 仅主 agent 可用。加类别要在 `route` 登记。
- `direct` 里的 web_fetch/update_plan/sub_agent_* 是占位/分派点，实际逻辑可能在上层（golish）以避循环依赖。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-kit tool_execution
```
