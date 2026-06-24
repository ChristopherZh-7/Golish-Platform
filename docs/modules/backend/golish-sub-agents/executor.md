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
| `response_parsing.rs` | tool call dispatch、stream chunk event、registry/router fallback attribution |
| `prompt_assembly` / `tool_setup` / `chain_persist` / `final_summary` | prompt / 工具 / 链持久化 / 末次总结 |

## 依赖

- crate 内 `definition`/`executor_types`/`executor_helpers`；`rig`、`golish-core::events`

## 注意事项 / 坑

- `timeout_secs=None` = 有进展就一直跑（靠 idle/per-tool/max_iterations 兜底）；改超时语义别让 sub-agent 永久挂起。
- 工具经 `ToolProvider` 注入（保持 L2 不反向依赖上层 runtime）；barrier 工具是 sub-agent 与主 agent 的交接点。
- `SubAgentExecutorContext.active_org_id_override` 是 stage-run per-org 硬隔离通道：registry fallback 执行 `manage_targets` / `manage_organizations` 时会注入内部隐藏 `__harness_org_id`，让工具按当前 org 子树过滤/绑定；不要把这件事退化成 prompt 约束。
- 普通 registry fallback 的 `Ok(Value)` 不是成功定义；必须用 `golish_core::utils::is_tool_result_success` 从 payload 判定。典型例子：WhatWeb 在 Ruby/OpenSSL 兼容问题下可能 `exit_code=0` 但 `stderr` 含 `ERROR Opening`，这要作为失败上报，UI 才能显示红色而不是绿勾。
- registry/router fallback 会用 `golish_core::with_agent_tool_context` 标记当前 sub-agent tool call；如果 `pentest_run` 等工具内部启动后台 shell，live chunk 要带 `ToolSource::SubAgent` 回到对应 sub-agent 工具详情。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-sub-agents executor
```
