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
| `SubAgentResult.chain_id` | 可选、serde 向后兼容的结构化 UUID；只指向已成功写入 provider-valid body 的 chain checkpoint |
| `SubAgentExecutorContext` / `SubAgentChainError` / `ToolProvider` / `BARRIER_TOOL_NAME`（re-export） | 执行上下文 / typed chain failure / 工具注入 / barrier |
| `BoundWorkerChainContext` / `BoundWorkerToolLifecycle` | server-owned V2 worker/chain/fence + optional whole-record resume source；阻止 model resume/fresh chain，并在每个 regular tool 前后做 awaited lifecycle |
| `SubAgentToolObserver` / `SubAgentToolObservation` | 上层 runtime 注入的工具结果观察点；包含真实 `tool_call_id`，可 trace-only，也可把纠偏提示附回 ToolResult |
| `SubmitRepairMode` / `SubmitRepairKind` / `submit_repair_mode_from_submit_result` / `submit_coverage_gap_repair_mode_from_reasons` / `refine_eas_web_repair_mode_from_worklist` / `retain_eas_web_repair_targets_for_same_gap` | 可持久化的 capability-first submit repair lock；runtime 的 StageRefiner 写入 checkpoint，executor resume 时恢复；EAS WEB exact lock 可从 DB worklist 确定性细化，并在同一 WEB gap 的重复 `needs_fix` 中保留 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | `execute_sub_agent` + re-export |
| `inner` | iterate-stream-dispatch 主循环 |
| `response_parsing.rs` | tool call dispatch、stream chunk event、registry/router fallback attribution |
| `prompt_assembly` / `tool_setup` / `chain_persist` / `final_summary` | prompt / 工具 / 链持久化 / 末次总结 |
| `history_compaction` | exact-chain restore 与每轮 provider preflight 的 deterministic history budget（pair-safe） |

## 依赖

- crate 内 `definition`/`executor_types`/`executor_helpers`；`rig`、`golish-core::events`

## 注意事项 / 坑

- `timeout_secs=None` = 有进展就一直跑（靠 idle/per-tool/max_iterations 兜底）；改超时语义别让 sub-agent 永久挂起。
- Prober/Enumerator 的 idle timeout 不能再充当 guarded EAS wrapper 的总墙钟截止：`eas_probe_http_liveness` / `eas_discover_ports` / `eas_fingerprint_services` / `eas_fingerprint_web_stack` 与既有 browser/JS/route direct tools 一样绕过 `tokio::time::timeout` 外层截断，继续由 shared cancellation 和工具自己的 bounded command timeout 收口。否则 drop future 会切断后续 authorized landing / evidence / outcome。
- `SubAgentExecutorContext.cancelled` 借用顶层 `AgentBridge` cancel flag；worker loop 在每轮开始、LLM stream 等待、工具 dispatch 前/等待工具时都要检查它。不要让 nested sub-agent 清掉该 flag，用户 Stop 必须能打断 stage_run 的 per-org worker。
- 工具经 `ToolProvider` 注入（保持 L2 不反向依赖上层 runtime）；barrier 工具是 sub-agent 与主 agent 的交接点。
- `SubAgentExecutorContext.active_org_id_override` 是 stage-run per-org 硬隔离通道：registry fallback 执行 `manage_targets` / `manage_organizations`，以及 Enumeration 的 `enum_crawl_same_origin_urls` / `browser_collect_js_api` / `js_extract_apis` / `route_probe_paths` 时都会注入内部隐藏 `__harness_org_id`，让工具按当前 org 子树过滤/绑定；不要把这件事退化成 prompt 约束。
- `SubAgentExecutorContext.operation_id` 是父 runtime 注入的可信 operation/stage-attempt identity。regular registry/router tool call 会把它和 `active_org_id_override` 一起写进 `AgentToolContext`，nested delegate 原值继承；它不能由模型可见参数覆盖或在子 agent 内重新生成。
- `SubAgentExecutorContext.persistence_session_id` 是 message-chain 的真实 DB session UUID；`session_id` 只用于事件/transcript。`chain_persist` 优先使用前者，legacy 裸 UUID event session 只作兼容回退。缺少两者时 fail closed，不创建不可归属的 chain，也不伪造可 resume marker。
- `bound_worker_chain=Some` 时 executor 必须忽略模型可见 `resume`，只 load exact bound worker/chain；其 `runtime_memory_source` 是 trusted request 选出的 server-owned 整源，`PgChainPersistence` 必须把它映射到 `LoadBoundWorkerChain.selected_source`，Preferred resume 不得在 executor/worker 级重新选择 V2 或 fallback。所有 initial/batch/final checkpoint 都走 `chain_checkpoint_bound_worker` compound seam，generic `chain_create/update/usage` 禁止。fresh claim 可把 objective 放进已 commit initial chain，executor 不得重复追加；普通 `bound_worker_chain=None` 路径保持原 legacy 行为。
- V2 regular tool（包括 unknown/可能有副作用的工具）必须经 `BoundWorkerToolLifecycle.begin/finish`。begin 失败不执行；finish/heartbeat/lease 失败不把 stale result 交给 post-result hook 或下一轮模型。prebound worker 禁止 nested delegation，以免同一 lease 出现两个 executor。
- exact resume 的 UUID 不是授权凭证：`chain_load_by_id` 必须带当前 session + agent scope；miss、load error、ownership mismatch、空/坏 history 都返回 typed error，不能 fallback latest/fresh。只有 chain body durable update 成功才可发布结构化 `SubAgentResult.chain_id` 或追加兼容 `[sub_agent_session_id: ...]` marker；update 失败必须让整个 worker 返回失败，usage 统计失败可在 body 已保存后仅告警。
- durable history 还必须满足 provider 的 tool-pair 不变量：每个 Assistant tool call 的下一条 User turn 都要含同 call id 的 ToolResult。fresh/exact/latest invocation 会在追加本次 user prompt 与恢复的 repair directive 后、任何 provider I/O 前（含 prompt-template generation）先写 initial provider-valid body；每个完整 Assistant tool-call batch + 对应 User ToolResult turn 再在 barrier / stage-stall / 下一次 provider request 前以单次 chain body update checkpoint。inner/outer 共享槽只在 update 成功后发布 UUID；graceful failure 和 outer timeout 只交还槽内最后成功快照，timeout drop 不做异步补写。initial snapshot 后的 generic `?` failure 会由 outer mapper 转为 `success=false + chain_id`；later `FinalizeFailed` 保持 typed/non-retryable，并把旧快照放在独立 `checkpointed_chain_id`，绝不把失败 update 的裸 chain id 当作新快照；initial update 失败时槽仍为空、原 Err 继续上抛。中间 checkpoint 不更新 usage，最终 teardown 只更新一次 usage。`submit_result` barrier 与 stage-stall 都必须先追加完整 result turn 再退出；同批 barrier 后的未执行 call 写明确 skipped result。serialize 写前和 exact/latest restore 后双向校验，任何普通工具缺口 fail closed，不能伪造执行结果。进程在 DB 确认 initial snapshot 前 hard-kill 不在该保证范围内。
- active Company Controller coordination turn（`bound_worker_chain.stage_team_leader=Some`）不得使用 generic `submit_result` 结束 executor：返回 `STAGE_TEAM_CONTROLLER_REQUIRES_ROUTER` 的非终态 ToolResult 后继续同一 durable chain，只允许 trusted `stage_team_dispatch_workers` / `stage_team_prepare_final_submission` 把控制权交还 scheduler。final submit turn 的 leader binding 已解除，因此不受该保护影响。
- ordinary durable Stage Team child 的 generic `submit_result` 不是自由文本出口：`tool_setup` 必须把 `result` 暴露为 exact `stage_worker_output.v1` object（`business_disposition/summary/fact_refs/evidence_ids/checked_empty_units/blocker_code`），`prompt_assembly` 同步要求 object 本体且禁止 Markdown/prose wrapper，`response_parsing` 再把该 object 序列化为单一 JSON object 交给 runtime 的确定性 validator。普通非 Stage Team sub-agent 仍使用 string result；Company Controller 与 Candidate verifier 保持各自 terminal contract。
- durable replay 不是“校验通过即可原样发送”：exact/latest restore 会先压缩 bulky tool results、折叠重复 repair directive，并把 provider-visible history 控制在 512 KiB；若发生变化要在 provider I/O 前写回同一 chain。loop 每次 `model.stream` 前与最终 persist 前复用同一 compactor；超过总量时只按完整 turn 单元淘汰最旧历史，绝不拆 Assistant tool call 与紧随的 User ToolResult；保留结果必须是连续的最新完整单元后缀，一旦较新单元放不下，不能再回填更旧的小单元形成历史空洞。
- sub-agent SSE item error 不是正常 EOF：必须返回 `success=false`，不得 dispatch 失败流里的 partial call，也不得把当前半轮推进为新 checkpoint；可以结构化返回此前已成功 checkpoint 的 UUID。若 stream-start 或 SSE error 明确是 input context 超限（含 DeepSeek `Request body has ... tokens ... limit` 400），必须返回带可选 checkpoint UUID 的 typed `ProviderContextLimitExceeded` 交由 runtime fail-stop；普通 400 / TPM rate limit 仍走普通失败。正常 text-only completion 则要写入 Assistant history，避免 continuation 丢掉 worker 结论。
- 普通 registry fallback 的 `Ok(Value)` 不是成功定义；必须用 `golish_core::utils::is_tool_result_success` 从 payload 判定。典型例子：WhatWeb 在 Ruby/OpenSSL 兼容问题下可能 `exit_code=0` 但 `stderr` 含 `ERROR Opening`，这要作为失败上报，UI 才能显示红色而不是绿勾。
- registry/router fallback 会用 `golish_core::with_agent_tool_context` 标记当前 sub-agent tool call；如果 `pentest_run` 等工具内部启动后台 shell，live chunk 要带 `ToolSource::SubAgent` 回到对应 sub-agent 工具详情，同时把 `active_org_id_override` 写入 tool context，确保后台 completion 落到当前 per-org worker 的 org。
- `response_parsing.rs` 对 sub-agent 的 `pentest_run` 结果以及带 `wrapped_tool_name` / `wrapped_args` 的 backend wrapper 结果也要触发 `post_shell_hook`（从 result/args 提取底层 `command/stdout`），否则 Prober/Enumerator 的 active scan/crawler 输出只进 evidence，不会自动走 output_store 写 `targets` / fingerprints / crawler endpoints。
- `stage_worklist_status` / `stage_worklist_next` / `check_stage_asset_coverage` 使用专用 model-visible compaction；Enumeration 的 200-cell 页会按 exact origin 聚合为最多 50 条 `exact_origin_page`（保留当前 `target_id`、`target_url`、base/root URL 和未完成 technique），避免总历史压缩只留下前几个 root 后让模型从旧页拼错 ID。模型必须从每项新建仅含 `{target_id,target_url}` 的严格 preflight 入参，不能复制整项或从历史重组。`enum_preflight_web_origins` 的即时与 durable-history 投影都无损保留最多 50 个 origin 的 `reachable_origins` / `blocked_origins` / `pending_origins` 分类，且每项只含严格 schema 的两个字段；raw details 留 transcript。`terminal_exceptions_preview.coverage_to_submit` 固定为空。
- `SubAgentToolObserver` 是 runtime 的泛型观察点：executor 只传工具名、参数、结果、成功状态，不反向依赖 harness/DB/LLM。当前 runtime 的 Mentor observer 已降级为 telemetry-only，不再把 advisor/supervisor 文本附回模型可见 ToolResult。
- 历史 hard-supervisor 同批 skip 逻辑仍保留为防御性兼容，但正常 repair/stage_run 路径不应再注入 `--- EXECUTION SUPERVISOR (HARD) ---`；模型可见纠错由 StageRefiner directive + `SubmitRepairMode` 提供。
- `response_parsing.rs` 写事件和 transcript 时保留工具真实返回的 `result_value`，但在 `UserContent::ToolResult` 回灌模型前会生成 model-visible compaction。大结果工具（route probe、enumeration web-root worklist、browser JS collect、JS extract）要保留 counts/samples/next_action，不要把完整 arrays 直接带进下一轮 sub-agent context。`route_probe_paths` single-root compaction 继续暴露 `matches_count` / `max_requests` / `request_limited` / `candidate_generation_limited`；batch 的 bridge 返回本身已是 bounded summary，model compactor 必须逐项保留最多 50 个 target 的 `target_id` / `base_url` / status/completion/outcome/persist/retry/checkpoint/queue/error diagnostics 与小样本，`retry.reason_codes` 始终保持完整 `string[]`，整体 serialized size 不得超过 512 KiB，不能再声称完整 raw arrays 留在 transcript。route v8 的 `automatic_retry_allowed=false` 是模型可见 retry 的最高优先级：compactor 必须保留 pending business-write/terminal cursor、authorization unavailable/superseded、两类 persistence breaker/counter、manual flag/reason，并输出 `retry.recommended=false` + 明确“停止自动重试；修复后用指定 flag 单 root 恢复”的 next_action，不能用 `partial || !queue_completed` fallback 把有限 breaker 重新变成无限 loop。browser batch compaction 必须为每个 root 保留小型 `root_diagnostics`（target_id/url/status/completion_state/closure reasons/page queue+resume counters），但不能把 raw scripts/API/results payload 回灌模型。`js_extract_apis` batch 同样必须逐 root 展开 `bounded_batch_summary_v1`，以 `endpoints_total` / `param_endpoints` 为真实计数，并为最多 50 个 accepted root 保留成功或失败的 status/completion/outcome/persist/retry/error diagnostics；禁止因 batch 顶层没有 `endpoints`、只有小样本或失败项位于第 10 项之后，就向模型报告 0 endpoints 或丢失待重试 root。
- `submit_stage_deliverable` 返回 `needs_fix` 且 gate 已给出 `available_evidence_ids` 时，`response_parsing.rs` 会先分类：纯 evidence/id 错误进入 evidence-ref repair-only（只允许 submit/query/wait），覆盖缺口（coverage / never attempted / EAS liveness/port/service/web-fingerprint / enumeration JSAPI/DIR/PARAM）进入 targeted gap-closure；该模式会持久化 `coverage_gap_actions`，把结构化 action list 注入模型指令，并在执行前拦截 `pentest_run` 的 CIDR/range、stdin/list-file、多目标 bulk probe；EAS coverage-gap repair 必须走 `eas_*` wrapper，raw `pentest_run` / raw `whatweb` 会被 block。若 action list 非空，还会阻止扫描未列出的 target；EAS wrapper batch 用 `targets[]` / `target_urls[]` 逐项校验，WEB-FINGERPRINT technique 即使没有 legacy `suggested_tools` 也要允许 `eas_fingerprint_web_stack`。WEB repair 先要求刷新 `stage_worklist_next` / `check_stage_asset_coverage`，把返回页的 DB-backed `recommended_args.target_urls`（兼容 `target_id + missing_origins`）持久化为 `eas_web_repair_targets` exact lock；object 必须精确匹配 `target_id + canonical origin`，bare string 也必须匹配 canonical origin，scheme/host/port 偏移均 fail closed。lock 一旦存在就不再采用 host-level action 放宽；刷新结果会立即更新同一 tool-call batch 的 effective mode，后续 wrapper 当批受新 exact lock 约束。bounded empty projection 不清锁，只有后端显式 `ready_to_submit=true` 才能把 exact set 关闭；runtime 同步把 refined lock 写入 checkpoint。相同 WEB action identity 的重复 `needs_fix` 继续保留该 lock，identity 变化则丢弃并要求刷新，不能因重新提交把 exact mode 降回 host-level。真正的 current-owner/exact-origin 校验仍由 wrapper 再次 fail-closed。enumeration action 会放行 direct tools（browser/js/js_extract/route_probe/enum_crawl_same_origin_urls）并校验 `target_url`/`target_urls`/`base_url`，crawler supplement 必须经 `enum_crawl_same_origin_urls`，不允许 raw `katana` 或 raw `pentest_run`。批次入参（design 2026-07-03：`target_urls` / `targets[].base_url`）在 coverage repair 下会**逐项**对照 `coverage_gap_actions`——任一 target 不在点名清单即整批 block，批次不能夹带未点名 target 越过 coverage-gap 围栏。coverage repair 即使带 StageRefiner `allowed_tools_override`，也必须保留只读 `stage_worklist_status` / `stage_worklist_next`，让 agent 能刷新当前 DB worklist；这不授权扫描 coverage_gap_actions 之外的 target。coverage needs_fix 若没有结构化 action list，则视为不能安全定位目标，只能 `stage_worklist_status` / `stage_worklist_next` / `check_stage_asset_coverage` / `query_target_data` / 等待后台 job / resubmit，禁止扫描和猜测新目标。
- `coverage_gap_actions` 指令要优先展示 `suggested_capabilities=<id...>`，再展示兼容的 `suggested_tools=<tool...>`；executor 只负责把 StageRefiner 的能力建议讲清楚并继续做目标围栏，不在本 crate 内决定 capability registry 或 DB truth。
- `submit_stage_deliverable` 返回 `needs_fix` 且仍有后台 job 未完成时，executor 会进入 wait-only repair lock：只允许 `wait_for_background_jobs` / `check_job` / `kill_job` / resubmit，避免模型开替代扫描把 UI 又变回 submit spinner。
- `submit_stage_deliverable` 返回 `accepted` 时，executor 把该工具结果当 terminal barrier：先持久化完整 assistant tool-call + ToolResult chain，再直接返回 orchestrator；同批后续工具标记 skipped。不能只清 `SubmitRepairMode` 后再请求一轮模型，否则 worker 会在已通过后重复 worklist/provider/submit。
- `SubAgentExecutorContext.initial_submit_repair_mode` 是 resume/refiner 入口：runtime 从 `agent_run.submit_repair_mode` 恢复后传入；executor 会把 directive 写进恢复后的 chat history，并发一条 SubAgentTextDelta 给 UI，随后用同一个 repair lock 继续拦截不允许的工具。`SubmitRepairMode` 支持 StageRefiner 覆盖 allowed/forbidden tools 和 directive 文案，用于 EAS/TargetIntel 的 stage-specific repair。
- `background:true` 工具若同步失败，也会提示不要把它当成运行中的后台 job。
- prebound Candidate worker 在 `BoundWorkerChainContext` 携带 opaque attempt ref；regular tool dispatch 在 generic tool lifecycle 前执行 core closed-tool guard，并把 ref 复制到 `AgentToolContext`，防止 raw/background/identity override 绕过。
- Candidate executor 的 authority 到 opaque Attempt 为止：它不接收 Wave generation/consolidation command，也不能接受或消费 FactDelta。`submit_candidate_attempt` 只是 terminal business submission；Attempt terminalization、VerificationUnit close 与 global Wave consolidation都由 executor 返回后的 runtime/orchestrator + DB compound seams完成，因此 agent text/tool success 不能替代这些终态。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-sub-agents executor
```
