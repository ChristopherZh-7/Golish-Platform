# golish-sub-agents / executor

> **一句话职责**：sub-agent 执行——`execute_sub_agent` 公开入口（可选总超时 + 统一错误处理），内层 iterate-stream-dispatch loop 在 `inner`，one-shot setup/teardown 分到 prompt_assembly / tool_setup / chain_persist / final_summary。

- **类型**：目录模块（属于 crate [`golish-sub-agents`](../golish-sub-agents.md)）
- **路径**：`backend/crates/golish-sub-agents/src/executor/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 sub-agent 执行循环、超时/空闲超时/max_iterations、barrier 工具时
- 改 prompt 组装（optimized + briefing + skills + barrier）、工具列表（allowed + barrier + 嵌套委派 shim）、消息链持久化时

## 职责

`execute_sub_agent` 包内层 orchestrator，加可选总超时 + 统一错误。`timeout_secs=None` 时 sub-agent 跑到完成（只受 idle/per-tool timeout + max_iterations 约束，有进展就继续）。内层 loop 在 `inner`，setup/teardown 分到子模块。

## 公开接口

| 符号 | 说明 |
|---|---|
| `execute_sub_agent` | 公开执行入口（超时 + 错误包装） |
| `SubAgentExecutorContext` / `ToolProvider` / `BARRIER_TOOL_NAME`（re-export） | 执行上下文 / 工具注入 / barrier |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `execute_sub_agent` + re-export |
| `inner` | iterate-stream-dispatch 主循环 |
| `prompt_assembly` / `tool_setup` / `chain_persist` / `final_summary` | prompt / 工具 / 链持久化 / 末次总结 |

## 依赖

- crate 内 `definition`/`executor_types`/`executor_helpers`；`rig`、`golish-core::events`

## 注意事项 / 坑

- `timeout_secs=None` = 有进展就一直跑（靠 idle/per-tool/max_iterations 兜底）；改超时语义别让 sub-agent 永久挂起。
- 工具经 `ToolProvider` 注入（保持 L2 不反向依赖上层 runtime）；barrier 工具是 sub-agent 与主 agent 的交接点。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-sub-agents executor
```
