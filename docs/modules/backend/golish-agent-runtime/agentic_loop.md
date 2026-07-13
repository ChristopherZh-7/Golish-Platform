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
| `AgenticLoopContext.runtime_memory` | 可选 V2 compound repository；stage worker 的 seed/claim/heartbeat/tool/chain fence 只走该句柄，eval/legacy 显式为 `None` |

## 关键文件

| 文件 | 作用 |
|---|---|
| `mod.rs` | 薄公开表面（≤150 LOC） |
| `turn/` | phase 调度本体（`run_turn_loop`） |
| `stream_processor/` / `tool_execution/` / `single_tool_call.rs` | 流式处理 / loop 内工具 / 单个 tool call 执行与 attribution |
| `worker_lease.rs` / `worker_tool_lifecycle.rs` | V2 worker 10s heartbeat/30s lease 与 durable tool row→begin/finish fence |

## 依赖

- crate 内（`golish-agent-kit` 经 crate 根）；`rig`（completion/streaming）

## 注意事项 / 坑

- `mod.rs` 是**薄表面**，逻辑在 `turn::*`——改 loop 行为去 turn，别把实现堆回 mod.rs。
- 透传宽 context（crate 级 `allow(too_many_arguments)`）是有意为之。
- `tool_execution/direct/mod.rs` 是 harness evidence 入账后的补充写点：recon passive 工具无论成功或失败都要写本轮 evidence/source attempt；成功时写 provider/RDAP typed terminal，失败时写 `error`（只证明尝试过，不能关 coverage），WHOIS 的 found/empty/error/blocked 不能被统一 success 文案覆盖。`recon_map_assets` 成功后还会触发 app bridge 标记已解析但无 DNS answers 的 in-scope domain 为 `GOLISH-INTEL-DNS empty`。target_intel duplicate guard 同时审查 generic action 与 `action:GOLISH-INTEL-*` 精确行；所有 relevant source rows 都是 found/empty/checked_empty/blocked，且对应 DNS technique outcome 不含 error/partial/running 时，才返回 `skipped_duplicate=true`。任何 retryable source/DNS outcome 都必须继续放行工具重试，与 stage spec 的 `error_is_terminal=false` 保持一致。
- `tool_execution/direct/mod.rs` 的 `run_pty_cmd` / `pentest_run` evidence 入账会派生通用 coverage facts（passive intel + EAS），并把 stdout+stderr 一起传给 outcome 判定；backend wrapper tools 通过 result 里的 `wrapped_tool_name` / `wrapped_args` 复用同一路径，所以模型看到的是 `eas_*` / `enum_crawl_same_origin_urls` / `vuln_run_formulaic_sweep` 工具，output-store/evidence 仍按底层 httpx/naabu/nmap/katana/nuclei/sqlmap/wpscan 命令解析。EAS 里 `nmap -sn` 的 DNS failure 只在 stderr 出现，不能只看 stdout。
- `guardrail_block_reason` 不得对 `submit_stage_deliverable` 的 evidence/claim 文本套 SSRF 或 dangerous-shell 字符串阻断：submit 只做确定性校验和 DB/control-plane 写入，不会 dereference subject；合法内网/loopback Red Team 目标必须能被记录。网络执行工具仍保留 guardrail，豁免不扩散到 probe/shell。
- enumeration 四轴（GOLISH-ENUM-JS/DIR/PARAM/JSAPI）的 evidence + technique outcome 现全部由 `golish-pentest-app/src/pentest_bridge` 工具自身通过 `append_bridge_evidence` 写入（design 2026-07-03）：`browser_collect_js_api` 落 JS + JSAPI、`js_extract_apis` 落 JSAPI + PARAM、`route_probe_paths` 落 DIR。这些工具在主 agent direct 路径和 enumerator 子 agent 路径都自负落账，因此旧的「只在主 agent direct 路径跑」的 `record_enumeration_bridge_evidence` runtime hook（及其 `enumeration_evidence_projections` 等 helper）已删除——子 agent 抓到 0 JS 时也能落 `GOLISH-ENUM-JS checked_empty`（I8），不再因为 hook 缺席而永远 `not_attempted`。bridge 工具自己的 `PentestAudit` 行只是 action timeline，不能当可引用 evidence。
- `sub_agent_call.rs` 会把同一套 recon passive evidence/source_query 记录逻辑作为 `SubAgentToolResultHook` 注入给 stage-run 子 agent，避免主 agent 和子 agent 工具路径落账口径漂移。`stage_run` per-org request id 形如 `...::org::<uuid>`；子 agent 的 evidence/source_query 写入必须优先使用该 org id，而不是继承父级 root org。
- `sub_agent_call.rs` 会把 `submit_stage_deliverable needs_fix` 先交给 `stage_refiner` 生成 `RepairDirective`，再把 directive + 兼容的 `SubmitRepairMode` 写入 `operation_state.state_blob.agent_run`（同 `agent_path`）：纯 evidence/id 场景恢复为 evidence-ref repair-only，`running_background_jobs` 场景恢复为 wait-only，coverage 缺口恢复为 targeted gap-closure；下一次 resume 该 sub-agent 时通过 `initial_submit_repair_mode` 注入 executor，并发一条 SubAgentTextDelta 说明正在恢复修复，不从头扫描。
- `sub_agent_call.rs` 还会把 `stage_run` per-org request id 解析成 `active_org_id_override` 注入子 agent；普通 registry 工具如 `manage_targets` / `manage_organizations` 会收到内部隐藏 org arg，所以 `action:"list"` 只能看该 org 子树，不能靠模型自己过滤 698 个全局资产。
- `stage_run_call.rs` 在 `harness_org_id` 已绑定时必须把 DB 中的 root organization subtree 当作 fan-out 权威集合：模型传入的 `orgs` 只是 ownership/name hint，续跑/修复轮如果少传子公司，runtime 要自动补回；如果传了 subtree 外 org，则记录并丢弃，避免串到 sibling/test engagement。
- V2-writing contract 的 `stage_run_call.rs` 不再读取 mutable subtree/model org 作为执行轴：先用 frozen scope `seed_stage_runtime` 得到完整 Unit/Worker，再 exact claim+prebound chain；Passed Unit 直接 skip，GateBlocked/expired-no-active-tool 恢复同 worker，active-tool expiry fail closed 为 recovery-required。每个 worker 自持 `WorkerLeaseSupervisor`，provider 前必须已 commit chain bind；普通非-stage sub-agent 保留 legacy `chain_create`。
- V2 specialist 工具必须先写 durable `tool_calls` row，再 `begin_worker_tool`；结果回灌/后处理前必须 `finish_worker_tool` 成功。heartbeat/begin/finish/checkpoint 共享 mutation lock 和 checkpoint-version witness；lease loss 会阻止下一工具并拒绝 stale landing。accepted submission 从当前 `ToolExecutionResult.value.response` 的 durable id reload，shared last-deliverable sink 仅供 legacy。
- V2 per-org Gate PASS 只表示“可申请封口”，不直接计入 stage PASS。`stage_run_call.rs` 必须按 stage 构造 server-owned seal material：`target_intel` / `external_attack_surface` / `enumeration` / `vuln_triage` 从 exact operation/org/wave coverage snapshot 聚合 terminal cells；`attack_candidate` 从 frozen manifest + server-classified `CandidateAcceptance` 构造 work-item refs、typed terminal decisions、decision evidence 与 `candidate_manifest_v1` watermark。Verification 走 Candidate compound claim + opaque verifier scheduler；模型只能调用 `submit_candidate_attempt` 写 immutable submitted result，scheduler 随即从 DB 重载 exact result/approval/evidence/action journal，并由单事务 terminalizer 写 terminal Attempt/Candidate、可选 Finding+lineage、FactDelta，再一起释放 WorkerRun/global lane。terminalizer 失败时保持 fail closed，绝不回退 legacy Finding writer。
- V2 wave-aware PASS 保持 Worker lease 到 compound close 提交：同一事务要么完成当前 wave、创建 exact child wave 并把 Worker/Unit 停到 `WaitingBackground`，要么完成 wave 后直接写 final seal；不得先发布 org completion。response-loss replay 只接受 exact completed parent/child/checkpoint/watermark 或 exact terminal wave，早期 completed wave 不能冒充最终封口。
- `submit_stage_deliverable` 的真实 result `status=accepted` 是两层确定性终止：sub-agent executor 把它当 barrier，主 agent `tool_dispatch` 也从完整 ToolResult JSON（允许后接 runtime note）识别并立即结束当前 harness loop。不能只记录“曾 dispatch submit”等待下一个 idle turn；`needs_fix/received` 仍继续正常修复/收口。
- `single_tool_call.rs` 对 `DbTracker::start_tool_call` / `finish_tool_call` 必须顺序 await：
  start row 可见后才 dispatch，finish 落库后才让 Scoping/gate 评分。否则非交互 CLI
  的 `scope_review` 会出现 finish 抢在 start 前、gate 偶发读不到批准的竞态。
- `single_tool_call.rs` 的自动 tool-result memory 入口按 server-owned `harness_stage` / `harness_operation_id` 计算 `LegacyToolMemoryContext`；任何 harness customer fact 都禁止进入 legacy `memories` 自由文本表，必须等待 canonical row + outbox Memory Fabric。
- C7 active harness 在 RuntimeSupervisor prompt 与 specialist briefing 前调用 `retrieve_scoped_context_data`：subject 必须同时具有 operation/stage execution/unit/org/stage identity，provider 再从 DB 重验 frozen ownership。返回内容经 data-only renderer 注入；identity/provider/retrieval 缺失或失败不得回退 legacy global memories/wiki。非-harness 普通 sub-agent 才保留原 legacy briefing。
- `stage_run_call.rs` 只在最终 per-org gate `PASS` 后物化 deliverable 中的 blocked/not_applicable：先重读同 org/session/epoch/wave 的 authoritative snapshot，只接受 exact 仍 pending/error/partial 的 asset×technique；Target Intel 的 organization context row可保存 WHOIS/ASN/OSINT 终态但永不成为 scan target。Found 与 checked_empty 不由模型写；repo 的条件 upsert 只插缺行或替换 partial/error，不降级 producer terminal truth。snapshot 或 upsert 失败必须把本次 org 反转为 BLOCK，不能 clear checkpoint/计 PASS；条件 upsert 返回 false 只表示并发 producer/既有 terminal truth 胜出，可安全继续。
- `sub_agent_call.rs` 把 `SubAgentResult.chain_id` 透传到 tool result；`stage_run_call.rs` 对未 PASS org 会在 `operation_state.state_blob.stage_run_workers[stage][org_id]` 保存 specialist 的精确 chain UUID。worker mapping 优先解析结构化 `chain_id`，字段缺失或无效才回退 response tail 的 legacy `[sub_agent_session_id: ...]` marker；重跑同 stage/org 时必须用这个 exact id resume，不能用 `resume:"latest"` 混续别的 org。DB-backed live stage_run 不能用 sub-agent 对话完成状态兜底 PASS：如果 specialist 没留下 accepted `StageDeliverable`（例如 `submit_stage_deliverable` 返回 `needs_fix` / 后台 job 未完成），该 org 必须 BLOCK 并进入 retry/gap，而不是计为 passed。
- `sub_agent_call.rs` 会把 `SubAgentChainError` 映射成稳定 `error_code + chain_failure_kind`；typed `ProviderContextLimitExceeded` 的可选、已 checkpoint UUID 同样进入 error JSON，stage worker 因而能在 non-retryable context-limit failure 后保存 exact mapping。later `FinalizeFailed` 也保持 `finalize` typed/non-retryable，但 error JSON 的 `chain_id` 只来自独立 `checkpointed_chain_id`（上一次成功 body snapshot），不使用本次失败 update 的诊断 ID；initial update 失败没有该字段。`stage_run_call.rs` 在 gate/worklist fallback 前应用安全策略。exact restore 失败只有在旧 exact id 仍在时才可重试该 id，fresh create 失败发生在工具前可有界重试，latest/finalize 失败为 non-retryable；provider input-context 超限固定映射为 `sub_agent_provider_context_limit_exceeded` + `context_limit`，无论是否已有 resume id 都 non-retryable，避免同一请求越重试越大。finalize 失败不得因缺 marker 被误当普通 BLOCK 后重开 fresh worker。
- `stage_run_call.rs` 的 resume-skip 只复用 `stage_run_id == 当前 operation UUID` 的 fresh `org_stage_completions`，并额外要求 completion 晚于本次 `operation_state.stage_started_at`；另一个并发 operation 或旧 session 的 completion 不能把 worker 短路。non-wave per-org PASS 与 wave atomic seal 都把当前 operation UUID 写入 `stage_run_id`；pass-token 生成及 orchestrator 最终重算必须同样过滤该 UUID。兼容表被并发 operation 覆盖最多造成安全的重跑/BLOCK，不能造成误 PASS。Continuation/repair 时 runtime 会把模型少传的 org 补回 authoritative subtree；这些 auto-added 但已 current-operation fresh PASS 的 org 在初始 UI seed 中必须直接发 `passed`，不要先发 `queued` 再等 serial loop 轮到它。
- `stage_run_call.rs` 调 per-org gate 时要把同一个 active-stage floor 与完整 `StageAssetWaveView(started_at,target_ids,asset_values)` 传入 `evaluate_org_stage_gate`；EAS 等 wave stage 优先使用 durable current batch 冻结 denominator，只有 repo 明确 NoWave 才回退 `stage_started_at` cutoff。读取/创建 wave 报错或 present wave membership 无效时，该 org 立即记 BLOCK/gap，禁止 warn 后降级 cutoff。当前 batch PASS 后只 mark wave completed，不提前写 completion；等所有当前 org batch 收口后，aggregate 阶段通过 repo 原子屏障统一 queue 该 operation/org/stage 尚未进入任何 wave 的 backlog（包括首波 limit 截断旧资产和运行中新资产），或在确无候选时于同事务发布 `org_stage_completions` 水位。只要 queue 出 supplemental wave 就暂不发 close pass token；下一次 `stage_run` 只处理这些 items。续跑时 fresh completion 只有在覆盖当前 running wave 的 `started_at` 时才能跳过；legacy backfilled wave 仅在没有 parent 且全部 item 早于该 pass 时可补 complete，supplemental wave 不能被第一轮 pass ledger 短路。
- `stage_run_call.rs` 的 coverage读取和 `evaluate_org_stage_gate` 都必须携带同一个 current operation UUID。Enumeration 因此只会应用本 operation 在 EAS 产生、经 producer+独立探针双重 evidence 验证的 exact-origin exclusion；漏传 UUID 会让 worklist与final gate denominator 漂移。
- `tool_dispatch.rs` 把包含 `submit_stage_deliverable` 的 assistant batch 按原顺序串行执行；一旦其结构化结果为 `status=accepted`，余下调用不再 dispatch，但必须为每个 call id 生成配对的 synthetic skipped ToolResult。该 outcome 传到 turn executor 后，active harness stage loop 当轮立即结束，不能在 accepted 后继续 manage target/worklist。
- `stage_run_call.rs` 生成 pass_token 时必须与 orchestrator closeout 一样，优先按 `harness_org_id` 的 organization subtree 读取 `org_stage_completions`，并同时匹配 current operation UUID 与 active-stage `stage_started_at` freshness floor；不能用全库 organizations、其他 operation 或旧 completion，否则 sibling run/workspace/test org 会污染当前 fan-out stage。
- `stage_run_call.rs` 还会在 per-org gate retry 之间调用 StageRefiner 并写 `operation_state.state_blob.agent_run`：`GateBlocked` checkpoint 保存上一次 gate feedback、attempt index、worker chain ref、last tool ref、`repair_directive`，以及从 directive 派生的 repair mode；同一 `agent_path` 恢复时从 pending correction + repair mode 继续定向补洞，不从阶段开头重跑。EAS WEB repair 成功读取 `stage_worklist_next` / `check_stage_asset_coverage` 后，`sub_agent_call.rs` 用真实 tool-call id 原位更新 checkpoint 中的 DB-backed exact lock，同时保留 `repair_directive`、runtime corrections 与 sibling state；同一 worker 的重复 submit `needs_fix` 和后续 stage retry 都复用 `retain_eas_web_repair_targets_for_same_gap`，仅在新旧 WEB action identity 集合一致时保留该 lock，缺口变化时丢弃旧锁并强制重新刷新，不能把已关闭的 origin 留在 targeted repair 授权里，也不能把仍有效的 exact lock 降回 host-level。即使没有 execution monitor，只要 operation DB sink 存在仍执行这次持久化。PASS / exhausted / resume-skip 会清理本 org 的 `agent_run` 槽。
- `StageRunReentryGuard` 在一个顶层 Task 请求（含该请求的外层 reflector passes）内按 stage 共享：只有某 org 真正用尽 `MAX_ORG_GATE_ATTEMPTS` 才关闭该 stage 的再次派发；同请求后续 `stage_run` 直接返回 `retry_budget_exhausted=true`，不得重新启动 specialist。该状态也经 bridge executor 暴露给 TaskOrchestrator，阻止 text-only/gate-BLOCK reflector 再启动整个 primary executor；最终 BLOCK 仍被记录。新的 GUI/headless 顶层请求 lease 才重置 guard，因此显式“继续”可沿 durable worker chain 合法续跑；未耗尽的 blocked retry 和 supplemental asset wave 不受影响。`SubmitRepairMode` 的新 exact WEB lock 只在 runtime literal/checkpoint 中做向后兼容透传，实际 DB worklist 提炼与工具围栏仍归 sub-agent executor。
- Enumeration 的 exact-origin worklist 可跨 8 页以上，且弱模型可能每完成一页就提前结束。runtime 对每个 worker segment 前后都从 `DbRepoProvider::stage_asset_coverage` 的 raw assets/coverage cells 重算 `unfinished = pending+error+partial` 及完整规范化 `(exact-origin, technique)` key set。成功但无 accepted StageDeliverable，或 deliverable 的 coverage gap key set 与 authoritative unfinished key set 完全相等时，只有 unfinished 严格下降才复用当前 org 的精确 chain；同数量不同 key 或 compact truncation 不得误判。工作续段上限为 `min(ceil(root_count/50)-1, 8)`，ready=true 另有独立且最多一次 submit-only continuation。capacity continuation 不增加 per-org gate attempt；混合 blocker 保留正常 gate repair。无进展、缺链、取消、空 denominator、读取失败或预算耗尽直接进入 exhausted/reentry breaker。
- `stage_run_call.rs` 的 EAS Prober objective 是 coverage-driven：先读
  `check_stage_asset_coverage` / `query_target_data`，再选最小 batch。模型只调四个
  `eas_*` wrapper，不看 raw httpx/naabu/masscan/nmap/WhatWeb/pentest_run；wrapper 强制
  foreground，在 guarded business/evidence/outcome 落库后返回。domain/url 只做 Host/SNI
  LIVENESS + exact-origin WEB；IP 先 PORT 后 per-open-port SERVICE；CIDR 行只做
  LIVENESS/PORT，child IP 下波做 SERVICE/WEB；wildcard 不进 active wrapper。
- depth-0 stage orchestrator 会在 active harness stage 里看到只读 coverage/target query 工具（`list_in_scope_targets` / `list_attack_surface_seeds` / `query_target_data` / `check_stage_asset_coverage` / `stage_worklist_status` / `stage_worklist_next`）；这些工具既要在 `tool_list` 暴露，也必须在 direct executor 走 `execute_security_analysis_tool`，不能只声明不路由，否则主 agent 会得到 `Unknown tool`。coverage/worklist 工具还需要透传当前 `harness_stage` / `harness_org_id` / `harness_operation_id`，这样它们才能默认按当前阶段和 `stage_started_at` 做提交前缺口预检/下一批 work item。`stage_run_call::build_org_objective` 会把 specialist worker 写成 worklist-first loop：先 `stage_worklist_status`，`ready_to_submit=false` 时再 `stage_worklist_next(prefer=["pending","error"])`，只处理 items 点名的 asset×technique cell；`check_stage_asset_coverage` 作为最终 compact sanity，不靠 `submit_stage_deliverable` 试错。
- `stage_run_call::build_org_objective` 会额外写入当前 stage 的 capability registry 摘要，要求 specialist worker 优先按 worklist item 的 `suggested_capabilities` 选择能力，再把 `suggested_tools` 当实现 hint；这仍只是指导，最终 PASS/BLOCK 只看 DB/gate truth。
- `stage_run_call::build_org_objective` 还会读取 bridge 透传的顶层 `SubAgentContext.original_request`，把它放在 deterministic objective 与 stage methodology 之间的 `TOP-LEVEL OPERATOR CONSTRAINTS` 数据块中：最多 4096 Unicode 字符，超长首尾保留且显式 `truncated=true`，正文用 JSON string 引用。该块优先级低于 stage/org/scope/tool/safety/evidence/gate 合同，只能收紧执行；不能靠文字改变 stage、增加 org/target、扩大 scope、放宽 read-only/exact-origin、启用禁用工具或伪造 terminal。它不参与 worker-chain key、checkpoint 或 `StageRunReentryGuard` 状态。
- `stage_run_call.rs` 里的 `vuln_scanner` 是 `vuln_triage` 的真实默认 stage sub-agent；runtime 直接派发 `sub_agent_vuln_scanner`，不再映射到 `sub_agent_pentester`。Legacy/dual contract 保留旧 `record_finding` 工具面；只有 runtime-memory 与 attack-execution 两份 operation-frozen contract 都是 `v2_only` 时，`sub_agent_call.rs` 才同时从可见工具列表隐藏并在 call-time guard 拒绝 `record_finding`，让公式化阶段只写 observation/evidence。任一 contract/authority lookup 失败按 V2 安全边界隐藏该 writer。
- Candidate specialist 路由同样要求两份 immutable contract 都是 `v2_only`：`attack_candidate` 才改派 reasoning-only `attack_analyst`，`verification` 才改派 foreground `candidate_verifier`；`attack_analyst` 必须映射到可 claim 的 durable Pentester worker 类型，不能只生成一个 scheduler 不认识的字符串。Legacy/dual 继续使用 stage spec 的旧 specialist/直接执行路径，不能因 additive V2 worker 改写既有 operation。
- `attack_candidate` 在 provider dispatch 前必须逐个 seeded Unit 调 `attack_v2_seed_candidate_manifest_for_unit`；repo 缺失、predecessor handoff/coverage 截断、canonical outcome 漂移或 manifest freeze 失败都立即终止 stage entry。模型开始推理前 manifest 已 exact frozen，不能在 wrapper 运行时边扫边扩分母。
- `tool_list.rs` 对带 effective specialist 的 active stage 会隐藏 primary 的直接干活工具（`recon_*`、`manage_targets`、`sub_agent_*`），保留 `stage_run`、`submit_stage_deliverable`、`manage_organizations`、只读查询以及后台 job 控制工具（`wait_for_background_jobs` / `check_job` / `kill_job`）。effective specialist 既包括静态 `StageSpec.specialist`，也包括从同 operation persisted 双 contract 动态解析的 Candidate V2 specialist；因此 V2 Verification primary 是 `stage_run` coordinator，而 Legacy/dual Verification 保留旧工具面。Candidate contract authority 缺失/读取失败时隐藏直接工作 fail closed，由 `stage_run` 重读并拒绝无 authority 的 dispatch。`target_intel` / `external_attack_surface` 这类 per-org 阶段必须先进 `stage_run`，但 submit barrier 报 pending background jobs 后也必须能由 main agent 收口旧 job，不能被 specialist-stage filter 挡住。
- `AgenticLoopContext.harness_forced_tool` 是比 prompt 更硬的 one-shot tool lock：completion 阶段会把 tool_choice 锁到指定工具并追加高优先级 directive，dispatch 阶段还会拒绝同一批里其他 allow-listed 工具。裸 resume 目前用它强制 first turn 调 `stage_run`；`stage_run` 用 `{"orgs":[]}` 让 `stage_run_call.rs` 从 bound engagement root 自动扩 authoritative subtree。
- `selection_apply.rs` 会按 `ExecutionModePolicy` 返回的 `StaticGroupSelection` 过滤静态工具目录；不要只用 `any_enabled()` 决定“是否全量加静态工具”。Task/Profile lead 依赖这个过滤来保留普通协作工具，同时排除 `security_analysis` 查询工具和 shell，强制真实安全运营请求经 `start_operation` 进入 harness。
- `first_iter_hooks.rs` 里的 reflector 开关要区分 Task/Profile lead 和 active harness stage：lead turn 没有 `harness_stage` 时必须允许普通文本回答/澄清，不要触发 reflector 去催模型“别等授权，开始执行”；进入真实 stage 后才恢复 task reflector 行为。
- `sub_agent_call.rs` 还负责给子 agent 注入非 `ToolRegistry` 工具路由：`list_in_scope_targets` / `list_attack_surface_seeds` / `query_target_data` / `check_stage_asset_coverage` / `stage_worklist_status` / `stage_worklist_next` 等 security-analysis read helpers 走 `execute_security_analysis_tool`，graph tools 走 graph executor，避免“工具已暴露但 UnknownTool”。
- `single_tool_call.rs` 会用 `golish_core::with_agent_tool_context` 包住主 agent 工具执行；context 同时携带可信 `harness_operation_id` 与 `harness_org_id`。后台 shell/pentest job 在启动时读取这个 task-local context，把 stdout/stderr chunk 归还到正确 tool card；主动 producer 也能按 operation/stage attempt + org 落 structured output / evidence / technique outcomes，而不接受模型参数伪造 identity。
- `sub_agent_call.rs` 的三条模型选择分支都把同一个 `harness_operation_id` 放进 `SubAgentExecutorContext`；sub-agent executor 再把它放进每个 registry/router 工具的 `AgentToolContext`，nested delegation 继续原值透传。漏掉任一构造分支会让主 agent 与 specialist worker 的 freshness/ownership 口径漂移。
- `sub_agent_call.rs` 的三条模型选择分支还必须把 `DbTracker::session_uuid()` 放进 `SubAgentExecutorContext.persistence_session_id`。`ctx.events.session_id` 仍是 trace/transcript 文本键，headless CLI 下形如 `stage-run-*`，不能当数据库 UUID；两者混用会让 Enumerator 正常完成一段工作后没有 `sub_agent_session_id`，从而错误触发 capacity exhaustion。
- `sub_agent_call.rs`、reflector、`stage_run_call.rs` 间接启动的 worker 必须把 `AgenticLoopContext.cancelled` 传入 `SubAgentExecutorContext`；ChatPanel Stop 置位后，main agent、stage_run per-org worker、nested sub-agent 都应共享同一个停止信号。
- `single_tool_call.rs` / `sub_agent_call.rs` 用 `ExecutionMonitor` 触发 RuntimeSupervisor：重复/停滞工具命中后调用一次 LLM，解析 JSON 为 `StrategyDirective`，再由 stage/tool policy 裁剪；`shadow` 只 trace，`soft/hard` 会把 RuntimeSupervisor directive 追加到 tool result，hard 模式会让 sub-agent executor 跳过同一批剩余工具调用。
- `HarnessTraceKind::StageRefinerDecision` 在 submit needs_fix / stage_run per-org BLOCK 时发出；`HarnessTraceKind::RuntimeSupervisorDecision` 在运行中策略监督触发时发出。`run_tree.py` 会显示二者的 kind/action/hash/root cause。
- `tool_execution/direct/mod.rs` 对 `pentest_run` registry 结果以及带 `wrapped_tool_name` / `wrapped_args` 的 backend wrapper 结果（EAS wrappers、`enum_crawl_same_origin_urls`、`vuln_run_formulaic_sweep`）都会触发 `PostShellHook`（使用结果里的 `command/stdout`，wrapper 则先还原底层命令），让 EAS active probes、enumeration crawler supplement 和 vuln formulaic sweeps 复用 `golish-pentest::output_store::maybe_detect_and_store_via` / background outcome listener 自动写 `targets` / fingerprints / `api_endpoints(source='crawler')` / `technique_outcomes`；不要只把 structured-storage hook 挂在 `run_pty_cmd`。在 `ctx.harness_stage.is_some()` 的 stage worker 内，这个 hook 必须 await，确保 submit preview / gate 评分前 DB truth 已落库；非 harness path 仍可 fire-and-forget。
- `PostShellHook` 参数为 `(command, stdout, project_path, organization_id)`：主 agent 用 `AgenticLoopContext.harness_org_id`，sub-agent 用 `active_org_id_override`。这个 org context 是 EAS/Enumeration 主动落库的资产归属来源，不能丢；丢了新资产会变成 `organization_id=NULL`，从 per-org gate denominator 和 stage_run 资产矩阵里消失。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-runtime agentic_loop
```
