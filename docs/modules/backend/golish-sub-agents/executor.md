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
| `SubAgentToolObserver` / `SubAgentToolObservation` | 上层 runtime 注入的工具结果观察点；可 trace-only，也可把纠偏提示附回 ToolResult |
| `SubmitRepairMode` / `SubmitRepairKind` / `submit_repair_mode_from_submit_result` / `submit_coverage_gap_repair_mode_from_reasons` | 可持久化的 submit repair lock；runtime 的 StageRefiner 写入 checkpoint，executor resume 时恢复 |

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
- `SubAgentExecutorContext.cancelled` 借用顶层 `AgentBridge` cancel flag；worker loop 在每轮开始、LLM stream 等待、工具 dispatch 前/等待工具时都要检查它。不要让 nested sub-agent 清掉该 flag，用户 Stop 必须能打断 stage_run 的 per-org worker。
- 工具经 `ToolProvider` 注入（保持 L2 不反向依赖上层 runtime）；barrier 工具是 sub-agent 与主 agent 的交接点。
- `SubAgentExecutorContext.active_org_id_override` 是 stage-run per-org 硬隔离通道：registry fallback 执行 `manage_targets` / `manage_organizations` 时会注入内部隐藏 `__harness_org_id`，让工具按当前 org 子树过滤/绑定；不要把这件事退化成 prompt 约束。
- 普通 registry fallback 的 `Ok(Value)` 不是成功定义；必须用 `golish_core::utils::is_tool_result_success` 从 payload 判定。典型例子：WhatWeb 在 Ruby/OpenSSL 兼容问题下可能 `exit_code=0` 但 `stderr` 含 `ERROR Opening`，这要作为失败上报，UI 才能显示红色而不是绿勾。
- registry/router fallback 会用 `golish_core::with_agent_tool_context` 标记当前 sub-agent tool call；如果 `pentest_run` 等工具内部启动后台 shell，live chunk 要带 `ToolSource::SubAgent` 回到对应 sub-agent 工具详情，同时把 `active_org_id_override` 写入 tool context，确保后台 completion 落到当前 per-org worker 的 org。
- `response_parsing.rs` 对 sub-agent 的 `pentest_run` 结果也要触发 `post_shell_hook`（从 result/args 提取 `command/stdout`），否则 Prober/Enumerator 的 active scan 输出只进 evidence，不会自动走 output_store 写 `targets` / fingerprints。
- `SubAgentToolObserver` 是 runtime 的泛型观察点：executor 只传工具名、参数、结果、成功状态，不反向依赖 harness/DB/LLM。当前 runtime 的 Mentor observer 已降级为 telemetry-only，不再把 advisor/supervisor 文本附回模型可见 ToolResult。
- 历史 hard-supervisor 同批 skip 逻辑仍保留为防御性兼容，但正常 repair/stage_run 路径不应再注入 `--- EXECUTION SUPERVISOR (HARD) ---`；模型可见纠错由 StageRefiner directive + `SubmitRepairMode` 提供。
- `response_parsing.rs` 写事件和 transcript 时保留 raw `result_value`，但在 `UserContent::ToolResult` 回灌模型前会生成 model-visible compaction。大结果工具（route probe、enumeration web-root worklist、browser JS collect、JS extract）要保留 counts/samples/next_action，并明确 `raw_result_retained_in_transcript=true`，不要把完整 arrays 直接带进下一轮 sub-agent context。
- `submit_stage_deliverable` 返回 `needs_fix` 且 gate 已给出 `available_evidence_ids` 时，`response_parsing.rs` 会先分类：纯 evidence/id 错误进入 evidence-ref repair-only（只允许 submit/query/wait），覆盖缺口（coverage / never attempted / EAS liveness/port/service / enumeration JSAPI/DIR/PARAM）进入 targeted gap-closure；该模式会持久化 `coverage_gap_actions`，把结构化 action list 注入模型指令，并在执行前拦截 `pentest_run` 的 CIDR/range、stdin/list-file、多目标 bulk probe；若 action list 非空，还会阻止扫描未列出的 target。enumeration action 会放行 direct tools 并校验 `target_url`/`base_url`，CLI 工具仍必须经 `pentest_run(tool_name=...)`。批次入参（design 2026-07-03：`target_urls` / `targets[].base_url`）在 coverage repair 下会**逐项**对照 `coverage_gap_actions`——任一 target 不在点名清单即整批 block，批次不能夹带未点名 target 越过 coverage-gap 围栏。coverage repair 即使带 StageRefiner `allowed_tools_override`，也必须保留只读 `stage_worklist_status` / `stage_worklist_next`，让 agent 能刷新当前 DB worklist；这不授权扫描 coverage_gap_actions 之外的 target。coverage needs_fix 若没有结构化 action list，则视为不能安全定位目标，只能 `stage_worklist_status` / `stage_worklist_next` / `check_stage_asset_coverage` / `query_target_data` / 等待后台 job / resubmit，禁止扫描和猜测新目标。
- `submit_stage_deliverable` 返回 `needs_fix` 且仍有后台 job 未完成时，executor 会进入 wait-only repair lock：只允许 `wait_for_background_jobs` / `check_job` / `kill_job` / resubmit，避免模型开替代扫描把 UI 又变回 submit spinner。
- `SubAgentExecutorContext.initial_submit_repair_mode` 是 resume/refiner 入口：runtime 从 `agent_run.submit_repair_mode` 恢复后传入；executor 会把 directive 写进恢复后的 chat history，并发一条 SubAgentTextDelta 给 UI，随后用同一个 repair lock 继续拦截不允许的工具。`SubmitRepairMode` 支持 StageRefiner 覆盖 allowed/forbidden tools 和 directive 文案，用于 EAS/TargetIntel 的 stage-specific repair。
- `background:true` 工具若同步失败，也会提示不要把它当成运行中的后台 job。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-sub-agents executor
```
