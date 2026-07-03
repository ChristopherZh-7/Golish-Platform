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
| `McpToolExecutor` / `OutputClassifier` / `PostShellHook` | 工具/分类/钩子；`PostShellHook` 会携带当前 `organization_id` |

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
- `tool_execution/direct/mod.rs` 是 harness evidence 入账后的补充写点：recon passive 工具拿到 evidence id 后，会把 provider/RDAP terminal rows 同步写入 `source_query_log`；`recon_map_assets` 还会触发 app bridge 标记已解析但无 DNS answers 的 in-scope domain 为 `GOLISH-INTEL-DNS empty`，避免 target_intel 把 checked_empty 画成 pending；target_intel 里同 run/action 已有 terminal source row 时会返回 `skipped_duplicate=true`，不再重复调用 provider。
- `tool_execution/direct/mod.rs` 的 `run_pty_cmd` / `pentest_run` evidence 入账会派生通用 coverage facts（passive intel + EAS），并把 stdout+stderr 一起传给 outcome 判定；EAS 里 `nmap -sn` 的 DNS failure 只在 stderr 出现，不能只看 stdout。
- enumeration 四轴（GOLISH-ENUM-JS/DIR/PARAM/JSAPI）的 evidence + technique outcome 现全部由 `golish-pentest-app/src/pentest_bridge` 工具自身通过 `append_bridge_evidence` 写入（design 2026-07-03）：`browser_collect_js_api` 落 JS + JSAPI、`js_extract_apis` 落 JSAPI + PARAM、`route_probe_paths` 落 DIR。这些工具在主 agent direct 路径和 enumerator 子 agent 路径都自负落账，因此旧的「只在主 agent direct 路径跑」的 `record_enumeration_bridge_evidence` runtime hook（及其 `enumeration_evidence_projections` 等 helper）已删除——子 agent 抓到 0 JS 时也能落 `GOLISH-ENUM-JS checked_empty`（I8），不再因为 hook 缺席而永远 `not_attempted`。bridge 工具自己的 `PentestAudit` 行只是 action timeline，不能当可引用 evidence。
- `sub_agent_call.rs` 会把同一套 recon passive evidence/source_query 记录逻辑作为 `SubAgentToolResultHook` 注入给 stage-run 子 agent，避免主 agent 和子 agent 工具路径落账口径漂移。`stage_run` per-org request id 形如 `...::org::<uuid>`；子 agent 的 evidence/source_query 写入必须优先使用该 org id，而不是继承父级 root org。
- `sub_agent_call.rs` 会把 `submit_stage_deliverable needs_fix` 先交给 `stage_refiner` 生成 `RepairDirective`，再把 directive + 兼容的 `SubmitRepairMode` 写入 `operation_state.state_blob.agent_run`（同 `agent_path`）：纯 evidence/id 场景恢复为 evidence-ref repair-only，`running_background_jobs` 场景恢复为 wait-only，coverage 缺口恢复为 targeted gap-closure；下一次 resume 该 sub-agent 时通过 `initial_submit_repair_mode` 注入 executor，并发一条 SubAgentTextDelta 说明正在恢复修复，不从头扫描。
- `sub_agent_call.rs` 还会把 `stage_run` per-org request id 解析成 `active_org_id_override` 注入子 agent；普通 registry 工具如 `manage_targets` / `manage_organizations` 会收到内部隐藏 org arg，所以 `action:"list"` 只能看该 org 子树，不能靠模型自己过滤 698 个全局资产。
- `stage_run_call.rs` 在 `harness_org_id` 已绑定时必须把 DB 中的 root organization subtree 当作 fan-out 权威集合：模型传入的 `orgs` 只是 ownership/name hint，续跑/修复轮如果少传子公司，runtime 要自动补回；如果传了 subtree 外 org，则记录并丢弃，避免串到 sibling/test engagement。
- `stage_run_call.rs` 对未 PASS org 会在 `operation_state.state_blob.stage_run_workers[stage][org_id]` 保存 specialist 的 `sub_agent_session_id`；重跑同 stage/org 时必须用这个精确 chain id resume，不能用 `resume:"latest"` 混续别的 org。DB-backed live stage_run 不能用 sub-agent 对话完成状态兜底 PASS：如果 specialist 没留下 accepted `StageDeliverable`（例如 `submit_stage_deliverable` 返回 `needs_fix` / 后台 job 未完成），该 org 必须 BLOCK 并进入 retry/gap，而不是计为 passed。
- `stage_run_call.rs` 的 resume-skip 可以复用 fresh `org_stage_completions`，但只要 `operation_state.current_stage` 等于当前 stage，就必须额外要求 completion 晚于本次 `operation_state.stage_started_at`；旧 session 的 completion 不能把 worker 全部短路，否则本轮没有新 evidence/source coverage，最终 submit 会被 deterministic gate 卡住。Continuation/repair 时 runtime 会把模型少传的 org 补回 authoritative subtree；这些 auto-added 但已 fresh PASS 的 org 在初始 UI seed 中必须直接发 `passed`，不要先发 `queued` 再等 serial loop 轮到它，否则用户会误以为已通过 worker 又被排队重跑。
- `stage_run_call.rs` 调 per-org gate 时要把同一个 active-stage `stage_started_at` floor 传入 `evaluate_org_stage_gate`；EAS 等 `asset_wave_barrier` stage 优先使用 durable `stage_asset_waves` 的当前 batch asset list 冻结 denominator，没有 durable wave 时才回退到 `stage_started_at` cutoff。当前 batch PASS 后只 mark wave completed 并写 `org_stage_completions`，不要在同一个 org 内自动 promote/continue 下一 wave；运行中新发现的 target 作为 `next_wave_pending` / expansion backlog 暴露。等所有 org seed batch 收口后，aggregate 阶段统一 queue 有新增资产的 org delta batch，并暂不发 close pass token；下一次 `stage_run` 处理这些 delta batches。续跑时如果 fresh `org_stage_completions.passed_at` 已覆盖当前 running wave 的 `started_at`，或 running wave 里的所有 target 都早于该 pass，completion 是权威事实源：先补齐 stale wave 的 completed 状态，再跳过重跑；如果 current wave 含 pass 之后的新 target，则继续跑当前未完成 batch。
- `stage_run_call.rs` 生成 pass_token 时必须与 orchestrator closeout 一样，优先按 `harness_org_id` 的 organization subtree 读取 `org_stage_completions`，并复用同一个 active-stage `stage_started_at` freshness floor；不能用全库 organizations 或旧 completion，否则其他 workspace/test org 或旧 pass ledger 会把当前 fan-out stage 卡死。
- `stage_run_call.rs` 还会在 per-org gate retry 之间调用 StageRefiner 并写 `operation_state.state_blob.agent_run`：`GateBlocked` checkpoint 保存上一次 gate feedback、attempt index、worker chain ref、last tool ref、`repair_directive`，以及从 directive 派生的 repair mode；同一 `agent_path` 恢复时从 pending correction + repair mode 继续定向补洞，不从阶段开头重跑。PASS / exhausted / resume-skip 会清理本 org 的 `agent_run` 槽。
- `stage_run_call.rs` 给 specialist objective 的后台任务语义必须与 submit barrier 一致：长扫后台化后不要重跑；`submit_stage_deliverable` 负责等待归因 job 落证据，只有明确卡死时才检查/kill。EAS Prober objective 是 coverage-driven，不是固定流水线：先用 `check_stage_asset_coverage` / `query_target_data` 理解当前资产和缺口，再选择最小有效 batch；禁止对原始 in-scope 大列表直接跑 broad `nmap -sV -iL`，domain/url 只做 LIVENESS，PORT/SERVICE 批次只能使用 IP/CIDR host，SERVICE-FINGERPRINT 只能基于确认开放端口的 host:port 分组。后台等待按 visible wait/check loop 执行：任一 job 完成时先读完成输出和落库 evidence；有 stdout/stderr 进展就继续等；idle 或批次明显过宽时 `check_job`/`kill_job` 并终态收口。
- depth-0 stage orchestrator 会在 active harness stage 里看到只读 coverage/target query 工具（`list_in_scope_targets` / `list_attack_surface_seeds` / `query_target_data` / `check_stage_asset_coverage` / `stage_worklist_status` / `stage_worklist_next`）；这些工具既要在 `tool_list` 暴露，也必须在 direct executor 走 `execute_security_analysis_tool`，不能只声明不路由，否则主 agent 会得到 `Unknown tool`。coverage/worklist 工具还需要透传当前 `harness_stage` / `harness_org_id` / `harness_operation_id`，这样它们才能默认按当前阶段和 `stage_started_at` 做提交前缺口预检/下一批 work item。`stage_run_call::build_org_objective` 会把 specialist worker 写成 worklist-first loop：先 `stage_worklist_status`，`ready_to_submit=false` 时再 `stage_worklist_next(prefer=["pending","error"])`，只处理 items 点名的 asset×technique cell；`check_stage_asset_coverage` 作为最终 compact sanity，不靠 `submit_stage_deliverable` 试错。
- `tool_list.rs` 对带 `StageSpec.specialist` 的 active stage 会隐藏 primary 的直接干活工具（`recon_*`、`manage_targets`、`sub_agent_*`），保留 `stage_run`、`submit_stage_deliverable`、`manage_organizations`、只读查询以及后台 job 控制工具（`wait_for_background_jobs` / `check_job` / `kill_job`）；`target_intel` / `external_attack_surface` 这类 per-org 阶段必须先进 `stage_run`，但 submit barrier 报 pending background jobs 后也必须能由 main agent 收口旧 job，不能被 specialist-stage filter 挡住。
- `AgenticLoopContext.harness_forced_tool` 是比 prompt 更硬的 one-shot tool lock：completion 阶段会把 tool_choice 锁到指定工具并追加高优先级 directive，dispatch 阶段还会拒绝同一批里其他 allow-listed 工具。裸 resume 目前用它强制 first turn 调 `stage_run`；`stage_run` 用 `{"orgs":[]}` 让 `stage_run_call.rs` 从 bound engagement root 自动扩 authoritative subtree。
- `selection_apply.rs` 会按 `ExecutionModePolicy` 返回的 `StaticGroupSelection` 过滤静态工具目录；不要只用 `any_enabled()` 决定“是否全量加静态工具”。Task/Profile lead 依赖这个过滤来保留普通协作工具，同时排除 `security_analysis` 查询工具和 shell，强制真实安全运营请求经 `start_operation` 进入 harness。
- `first_iter_hooks.rs` 里的 reflector 开关要区分 Task/Profile lead 和 active harness stage：lead turn 没有 `harness_stage` 时必须允许普通文本回答/澄清，不要触发 reflector 去催模型“别等授权，开始执行”；进入真实 stage 后才恢复 task reflector 行为。
- `sub_agent_call.rs` 还负责给子 agent 注入非 `ToolRegistry` 工具路由：`list_in_scope_targets` / `list_attack_surface_seeds` / `query_target_data` / `check_stage_asset_coverage` / `stage_worklist_status` / `stage_worklist_next` 等 security-analysis read helpers 走 `execute_security_analysis_tool`，graph tools 走 graph executor，避免“工具已暴露但 UnknownTool”。
- `single_tool_call.rs` 会用 `golish_core::with_agent_tool_context` 包住主 agent 工具执行；后台 shell/pentest job 在启动时读取这个 task-local context，把 stdout/stderr chunk 归还到正确的 tool card，并携带当前 `harness_org_id` 让 completion listener 能按 org 落 structured output / technique_outcomes。
- `sub_agent_call.rs`、reflector、`stage_run_call.rs` 间接启动的 worker 必须把 `AgenticLoopContext.cancelled` 传入 `SubAgentExecutorContext`；ChatPanel Stop 置位后，main agent、stage_run per-org worker、nested sub-agent 都应共享同一个停止信号。
- `single_tool_call.rs` / `sub_agent_call.rs` 用 `ExecutionMonitor` 触发 RuntimeSupervisor：重复/停滞工具命中后调用一次 LLM，解析 JSON 为 `StrategyDirective`，再由 stage/tool policy 裁剪；`shadow` 只 trace，`soft/hard` 会把 RuntimeSupervisor directive 追加到 tool result，hard 模式会让 sub-agent executor 跳过同一批剩余工具调用。
- `HarnessTraceKind::StageRefinerDecision` 在 submit needs_fix / stage_run per-org BLOCK 时发出；`HarnessTraceKind::RuntimeSupervisorDecision` 在运行中策略监督触发时发出。`run_tree.py` 会显示二者的 kind/action/hash/root cause。
- `tool_execution/direct/mod.rs` 对 `pentest_run` registry 结果也会触发 `PostShellHook`（使用结果里的 `command/stdout`），让 EAS active probes 复用 `golish-pentest::output_store::maybe_detect_and_store_via` 自动写 `targets` / fingerprints；不要只把 structured-storage hook 挂在 `run_pty_cmd`。在 `ctx.harness_stage.is_some()` 的 stage worker 内，这个 hook 必须 await，确保 submit preview / gate 评分前 `targets.ports` / `fingerprints` 已落 DB；非 harness path 仍可 fire-and-forget。
- `PostShellHook` 参数为 `(command, stdout, project_path, organization_id)`：主 agent 用 `AgenticLoopContext.harness_org_id`，sub-agent 用 `active_org_id_override`。这个 org context 是 EAS/Enumeration 主动落库的资产归属来源，不能丢；丢了新资产会变成 `organization_id=NULL`，从 per-org gate denominator 和 stage_run 资产矩阵里消失。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-runtime agentic_loop
```
