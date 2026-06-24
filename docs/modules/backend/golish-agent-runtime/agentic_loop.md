# golish-agent-runtime / agentic_loop

> **一句话职责**：每-turn 状态机的公开入口（薄表面 ≤150 LOC）——`run_agentic_loop*`；真正的 phase 调度在 `turn::run_turn_loop`，子模块托管各 phase 调用的 helper 与工具集成（HITL 审批 / loop detection / 上下文窗口 / 消息历史 / extended thinking 流式）。

- **类型**：目录模块（属于 crate [`golish-agent-runtime`](../golish-agent-runtime.md)）
- **路径**：`backend/crates/golish-agent-runtime/src/agentic_loop/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改流式 tool-call loop 主体（turn 调度、stream_processor、tool_execution、sub_agent_dispatch、compaction）时
- 改 loop 入口签名（`run_agentic_loop_unified` 等）或产物目录解析时

## 职责

agentic loop 的公开表面 + 子模块实现。`turn/` 是 phase 调度本体；`stream_processor/` 处理流式；`tool_execution/` loop 内工具；`sub_agent_dispatch` 派发 sub-agent；compaction 触发上下文压缩。

## 公开接口

| 符号 | 说明 |
|---|---|
| `run_agentic_loop` / `_generic` / `_unified` | loop 入口 |
| `AgenticLoopConfig` / `AgenticLoopContext` / `LoopLlmRefs` / `LoopEventRefs` / `LoopCaptureContext` / `LoopAccessControl` | 配置 + 依赖注入束 |
| `maybe_compact` / `apply_compaction` / `CompactionResult` | 压缩 |
| `get_transcript_dir(_for)` / `get_artifacts_dir(_for)` / `get_summaries_dir(_for)` | 产物目录 |
| `McpToolExecutor` / `OutputClassifier` / `PostShellHook` | 工具/分类/钩子 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | 薄公开表面（≤150 LOC） |
| `turn/` | phase 调度本体（`run_turn_loop`） |
| `stream_processor/` / `tool_execution/` / `single_tool_call.rs` | 流式处理 / loop 内工具 / 单个 tool call 执行与 attribution |

## 依赖

- crate 内（`golish-agent-kit` 经 crate 根）；`rig`（completion/streaming）

## 注意事项 / 坑

- `mod.rs` 是**薄表面**，逻辑在 `turn::*`——改 loop 行为去 turn，别把实现堆回 mod.rs。
- 透传宽 context（crate 级 `allow(too_many_arguments)`）是有意为之。
- `tool_execution/direct/mod.rs` 是 harness evidence 入账后的补充写点：recon passive 工具拿到 evidence id 后，会把 provider/RDAP terminal rows 同步写入 `source_query_log`；target_intel 里同 run/action 已有 terminal source row 时会返回 `skipped_duplicate=true`，不再重复调用 provider。
- `sub_agent_call.rs` 会把同一套 recon passive evidence/source_query 记录逻辑作为 `SubAgentToolResultHook` 注入给 stage-run 子 agent，避免主 agent 和子 agent 工具路径落账口径漂移。`stage_run` per-org request id 形如 `...::org::<uuid>`；子 agent 的 evidence/source_query 写入必须优先使用该 org id，而不是继承父级 root org。
- `sub_agent_call.rs` 还会把 `stage_run` per-org request id 解析成 `active_org_id_override` 注入子 agent；普通 registry 工具如 `manage_targets` / `manage_organizations` 会收到内部隐藏 org arg，所以 `action:"list"` 只能看该 org 子树，不能靠模型自己过滤 698 个全局资产。
- `stage_run_call.rs` 对未 PASS org 会在 `operation_state.state_blob.stage_run_workers[stage][org_id]` 保存 specialist 的 `sub_agent_session_id`；重跑同 stage/org 时必须用这个精确 chain id resume，不能用 `resume:"latest"` 混续别的 org。DB-backed live stage_run 不能用 sub-agent 对话完成状态兜底 PASS：如果 specialist 没留下 accepted `StageDeliverable`（例如 `submit_stage_deliverable` 返回 `needs_fix` / 后台 job 未完成），该 org 必须 BLOCK 并进入 retry/gap，而不是计为 passed。
- `stage_run_call.rs` 给 specialist objective 的后台任务语义必须与 submit barrier 一致：长扫后台化后不要重跑；`submit_stage_deliverable` 负责等待归因 job 落证据，只有明确卡死时才检查/kill。
- depth-0 stage orchestrator 会在 active harness stage 里看到只读 coverage/target query 工具（`list_in_scope_targets` / `list_attack_surface_seeds` / `query_target_data`）；这些工具既要在 `tool_list` 暴露，也必须在 direct executor 走 `execute_security_analysis_tool`，不能只声明不路由，否则主 agent 会得到 `Unknown tool`。
- `sub_agent_call.rs` 还负责给子 agent 注入非 `ToolRegistry` 工具路由：`list_in_scope_targets` / `list_attack_surface_seeds` / `query_target_data` 等 security-analysis read helpers 走 `execute_security_analysis_tool`，graph tools 走 graph executor，避免“工具已暴露但 UnknownTool”。
- `single_tool_call.rs` 会用 `golish_core::with_agent_tool_context` 包住主 agent 工具执行；后台 shell/pentest job 在启动时读取这个 task-local context，把 stdout/stderr chunk 归还到正确的 tool card。
- `single_tool_call.rs` 还承接 `ExecutionMonitor` mentor 建议：`GOLISH_EXECUTION_MENTOR=shadow` 时调用 mentor 但只写 `harness::mentor` tracing，`soft`/`on` 才把 `--- EXECUTION ADVISOR ---` 追加进工具响应；不要绕过 hard gate/submit 规则。
- `tool_execution/direct/mod.rs` 对 `pentest_run` registry 结果也会触发 `PostShellHook`（使用结果里的 `command/stdout`），让 EAS active probes 复用 `golish-pentest::output_store::maybe_detect_and_store_via` 自动写 `targets` / fingerprints；不要只把 structured-storage hook 挂在 `run_pty_cmd`。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-runtime agentic_loop
```
