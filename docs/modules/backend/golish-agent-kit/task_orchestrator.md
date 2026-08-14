# golish-agent-kit / task_orchestrator

> **一句话职责**：harness 驱动的自动化任务执行——一个 Task 由 metalcraft Executor 在 profile 投影的 Operation DAG 上推进：每阶段自规划+派发 specialist、提交 StageDeliverable、过确定性 evidence gate 才前进；常规人工确认只在 Scoping，后续阶段经专用安全 barrier 后自动推进，末尾 reporter 收尾。

- **类型**：目录模块（属于 crate [`golish-agent-kit`](../golish-agent-kit.md)）
- **路径**：`backend/crates/golish-agent-kit/src/task_orchestrator/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 Task 模式编排（stage 自规划/派发/gate/前进、reporter 收尾）时
- 改 `AgentExecutor` trait、`TaskOrchestrator::run` 入口、per-stage agentic loop 时
- 改 operation/task 断线恢复、runtime correction/refiner checkpoint、`operation_state.state_blob` schema 时

## 职责

在 `AgentBridge` 之上编排整个 Task 生命周期 + DB 持久化，每次 agent 调用回落到 bridge。`subtask_phases` 跑 Executor 驱动的 operation loop + `execute_single_subtask`（per-stage loop + gate）；V2 Verification 在 Gate PASS 后还必须重读 exact DB truth并提交 operation-wide Wave consolidation，graph 只消费已提交的 durable decision。

## 公开接口

| 符号 | 说明 |
|---|---|
| `TaskOrchestrator`（`run`） | 编排主体 + 入口 + 事件发射；fresh run 接 trusted `ProjectScopeRegistration`，并经 `RuntimeMemoryRepository` 原子创建 task/operation |
| `TaskOrchestrator::set_cli_runtime_scope` | headless fresh-only trusted seam；把一次解析的 root/descendants/threshold 交给 compound create，`run_from_stage` 消费后清空，resume 永不重建 scope |
| `TaskOrchestrator::set_current_invocation_target_authority` | typed fresh-launch 三态 seam：`Some(true)` 允许 pre-EAS 读取并验证本次 exact target DB truth；`Some(false)` 在读历史 org targets 前直接 HOLD；GUI/unconfirmed fresh lifecycle 可保持 `None`。headless exact resume 不依赖 marker 持久化：present 时严格解析，missing 收紧为 `Some(false)`，malformed 拒绝 |
| Scoping V2 finalization | trusted Scoping gate PASS 后、`stage_passed`/阶段转换前调用 `RuntimeMemoryRepository::finalize_scoping_scope`；写前校验 operation 的预绑定 org 与 gate-approved root 一致，从 trusted submission 确定性派生 snapshot/root Unit UUID，验回 exact operation/project/execution/root/submission；首次写、幂等重放或任一身份/存储失败都必须先收紧为 BLOCK |
| `set_resume_runtime_memory_source` / `set_resume_task_preclaimed` | trusted resume seam：把完整 `Legacy` / `V2` / `LegacyFallback` source 固定给 graph checkpointer，并一次性消费调用侧已完成的 exact `waiting -> running` CAS；trusted caller 同时把该 source 注入 `AgentBridge`，使 child runtime reads 与 graph 同源且不再二次更新 task status |
| `AgentExecutor`（trait） | 每次 agent 调用的抽象（bridge 实现在 `golish-agent-bridge::bridge_executor`） |
| `agent_run_checkpoint` | P2a 细粒度 agent-run checkpoint DTO + `state_blob.agent_run` merge helpers |
| `stage_refiner` | Stage-aware deterministic repair owner：submit/gate 失败后生成 capability-first `RepairDirective`，并转换为 sub-agent `SubmitRepairMode` |
| `runtime_supervisor` | PentAGI-style in-run strategy supervisor：重复/停滞工具触发后解析 LLM JSON，按 stage/tool policy 裁剪成 `StrategyDirective` |
| `two_level_phase_gate` | graph-flow 流转闸：先执行 Candidate V2 review 与 TargetIntel→EAS exact target-scope barrier；其它 post-Scoping stage 在 Gate PASS 后自动推进，不再打开 generic phase confirmation |
| `hypothesis_analysis::{HypothesisAnalysisAgentRunner,HypothesisAnalysisRuntimeRepository,HypothesisAnalysisStageRuntime}` | Plan B submit-only model runner、durable repository与两波stage runtime ports；closed DTO含semantic summary，`AnalysisArtifactsReady`明确不是PASS |
| `types`（planning DTO / token usage / 执行上下文） | 编排类型 |
| `prompts` | 各阶段 prompt 模板 |

## 关键文件

| 文件 | 作用 |
|---|---|
| `orchestrator.rs` | `TaskOrchestrator` + `run` |
| `agent_run_checkpoint.rs` | resumable agent-run 状态：pending correction、background job ids、last tool ref、`submit_repair_mode`、`repair_directive` |
| `continuity.rs` | DB-backed continuity preflight：只读 in-scope org + `org_stage_completions`，构建可确认复用的 `ContinuityAdoptionPlan` |
| `stage_refiner.rs` | `RepairDirective` DTO + deterministic StageRefiner：EAS coverage gaps / TargetIntel provider-only repair / background/evidence refs |
| `runtime_supervisor.rs` | `StrategyDirective` DTO + RuntimeSupervisor prompt/parse/sanitize：运行中防跑偏，不决定 gate |
| `subtask_phases/` | Executor loop + `execute_single_subtask`（per-stage gate） |
| `types.rs` / `prompts/` / `helpers.rs` | DTO/trait · prompt 模板 · 共享小函数 |

旧 `task_orchestrator/verification_campaign.rs` 的operation-global route、stage closure与stall reducer没有production caller，已物理删除；新 Investigation 的公司/资产调度与每hypothesis终态不在TaskOrchestrator内维护第二套Campaign状态机。

## 依赖

- crate 内 `harness`（gate/DAG）、`AgentBridge`（经 trait）；`golish-events`

## 注意事项 / 坑

- **不变量 I7/I8**：每阶段必须过 evidence gate 才前进；gate 是确定性规则，不能拿「agent 自信说完成」当通过。
- `submit_stage_deliverable` 返回 accepted 后，executor 捕获的 canonical deliverable 是该 turn 唯一 Gate 输入；模型随后输出的自然语言只是展示，不得因缺少第二份 JSON block 覆盖或抹掉已接受提交，也不得再触发 text-only repair turn。Gate 随后仍必须按 operation/execution/unit/hash 重读 immutable submission row，不能把 side-channel capture 本身当成持久化 authority。
- 常规 HITL 只属于 Scoping。`two_level_phase_gate` 仍先执行 exact Candidate review 与主动目标范围授权，随后对所有 post-Scoping 普通 crossing 返回 `Allowed`；这不等于自动批准 Candidate、扩大 scope、批准高风险 tool call 或绕过 Gate BLOCK。`phases.json.entry_approval` / profile approval policy 保留为风险与兼容元数据，前端历史 confirmation renderer 和 CLI flag 也无需删除。
- Candidate contract routing 必须同时读取 operation persisted `RuntimeMemoryContract` 与 `AttackExecutionContract`，但 synthesis、blocking review 与 verifier 分开 gate：`attack_candidate` 在 runtime-memory 可写 V2 且 attack contract `writes_v2()` 时进入 relational `attack_analyst` 路由，覆盖两个 dual contract并产生真实 shadow sample；review barrier 和 `candidate_verifier`/exact DB truth 只有两者都为 `v2_only` 才启用，Legacy/dual 不读 barrier、不 hold、不执行 verifier。同一 effective-specialist 解析用于 fresh stage prompt 与裸 resume 的 `stage_run` one-shot force。contract lookup 失败时 Candidate primary 留在 specialist-only 路由，交由 `stage_run` 权威重读 fail closed。Legacy/dual Verification 保留旧 deliverable candidate 的 bounded `chain_wave` fallback。V2 Gate PASS 后 `resolve_stage_flow_outcome` 重读并校验同 operation/scope/current-wave 全部 Unit truth，再调用 `attack_v2_consolidate_wave`；只有返回 identity/decision/count shape 全部匹配才发安全的 `AttackWaveConsolidated` trace，并以 `durable_wave_cursor=true` 把 durable终态交给 graph：`opened_next_wave` 回 AttackCandidate，`exhausted` 固定 Reporting，`closed_no_delta` 只有 proof-backed + Finding-linked exact `verified` Attempt 才走 AccessValidation，零 Candidate / 全 refuted / blocked 都走 Reporting。`pending_enrichment` 是显式可观察 BLOCK：trace 带 pending count，source Wave 不推进，不能当 invalid/error 或伪造下一 Wave。missing/extra/DB error/foreign/inconsistent/consolidation failure 一律改判 BLOCK，不能回退进程内 wave state。
- Reporting stage entry 先通过 `DbRepoProvider::reporting_build_validated_revision` 从完整 canonical DB source set build/reuse validated revision；失败时在任何 agent turn 前 BLOCK。stage close 再用 `reporting_gate_truth` 重读 current truth。Reporting 跳过 generic enrichment/planner/wiki prior，只允许最小 StageDeliverable；stage seam 不持有 artifact/finalizer 能力，最终发布只能走显式本地 operator command。
- `bridge_executor`（`AgentExecutor` 实现）在 `golish-agent-bridge`（依赖 AgentBridge）；本模块只持 trait。
- graph-flow 的 `operation_state.current_stage` 表示**当前正在执行的 stage**：进入新 stage 时同步并刷新 `stage_started_at`；断线后回到同一 stage 时不能重复刷新，否则 freshness-window gate 会看不到断线前已落库的 evidence。
- `operation_state.state_blob` 是 graph checkpoint、stage_run worker resume 等多消费者共享 JSON；更新 `graph_flow` 时要 merge 保留其他 key，不能整段覆盖。
- 上一条只适用于整源选择 legacy 的 resume。`V2Only` 禁止 initial flat HarnessResume 写入；`DualWriteV2Preferred` resume 必须由 trusted caller 显式传入 `V2` 或 `LegacyFallback`。V2 source 只从 relational `current_stage` 启动默认进程内 graph state，不读取、合成或回写 `graph_flow`。`resume_task_preclaimed` 是 one-shot handoff，`resume()` 必须 `take` 后跳过一次 generic status update，不能让后续请求继承。
- `operation_state.state_blob.agent_run` 是 P2 细粒度恢复槽：只存稳定、可恢复的轻量状态（如 pending gate correction、submit repair directive、background job ids、last tool result ref），大段输出仍留 transcript/background job/evidence ledger。写入或清理时必须保留 `graph_flow` / `stage_run_workers` 等 sibling key。`repair_directive` 是 StageRefiner 的结构化真相源，`submit_repair_mode` 是给 executor/tool guard 的兼容投影。
- Cross-session adoption 不等于 resume：resume 继续同一个 `graph_flow` checkpoint；adoption 是新 operation 经用户确认后复用旧 DB truth。`TaskOrchestrator::set_continuity_adoption` 会同时设置 entry stage 和 remaining-stage allowlist，否则 metalcraft Executor 仍会从 profile DAG entry（通常 scoping）启动。
- continuity preflight 不能在没有 `engagement_root` 的情况下 adopt `scoping`：legacy `in_scope_org_ids(None)` 可能包含 sibling/test org，跳过 scoping 会让后续 pass-token/coverage/gate 落回全库口径并污染当前任务。
- TaskMode 必须先把 chat key 解析出的 durable `sessions.id` 绑定到所有共享 `DbTracker` clones，再构造/运行 stage executor。`TaskOrchestrator.session_id`、gate 的 session 查询和工具生命周期写入必须同值；随机 tracker UUID 不能进入 stage，否则真实 `unit_review` / `scope_review` 也会因查不到 lifecycle 而被反复 BLOCK。
- Fresh operation 不再由 orchestrator 分两步写 task/operation，也不存在 runtime-contract setter；它把 server UUID、profile、entry stage 与 trusted project scope 交给 compound repository。headless CLI 可额外通过 `set_cli_runtime_scope` 附加已经解析的 trusted flags scope；该值只在下一次 fresh create 被 `take()` 一次，并与 task/operation/initial execution/decision/snapshot 原子提交，不能进入 resume 或由模型输入构造。原子创建或 project identity 失败时必须在任何 LLM/provider 调用前返回；resume 只读原 operation 的 frozen contract/scope，当前 workspace scope 不匹配时拒绝重绑。
- Red-team scoping 的 anti-shortcut gate 是两分支：同一 trusted root 的 persisted subsidiary choice 明确 parent/root-only 时，不能再制造空 `propose_candidates` / `unit_review`；选择纳入时才强制成功 `propose_candidates` 后接同 org、non-skipped、可解析的 `unit_review`。不再强制 `manage_organizations(create)`：已有 root/org tree 经用户确认后就是可复用记录，不能为了满足 gate 再 `create_batch` 扩树。只有 root 缺失或用户在 unit review 里显式新增/确认的单位才创建。
- Red-team `scope_review` 只能审 trusted UI/CLI 在 stage 前落库的 exact target
  snapshot；trusted intake 包含 `customer_provided`，但 `discovered` / `asset_intel` /
  provider-derived sources 不是授权根。snapshot 为空表示 organization-only engagement，
  不要求制造空 `scope_review`；snapshot 非空才必须有 exactly one 成功、可解析且与
  canonical value + type + scope 完全一致的 review。gate 消费本轮全部 persisted review
  lifecycle，第二次确认不能洗掉第一次编辑/拒绝。skip/free text/编辑 proposal/读库失败都
  fail closed；Scoping/Target Intel 不暴露 `manage_targets`，org profile 不能替代 seed。
- Active recon 入口不是普通 HITL：direct EAS stage request 与“Target Intel 已 PASS、图 cursor 已请求 EAS、但 scope review 尚未完成”的恢复入口共用 review-capable guard。direct EAS 没有 current Target Intel candidate window，因此仍只接受既有 trusted-target authority；恢复入口则必须先发 operation-bound `scope_review`，不能让只读 trusted-target preflight 抢先 HOLD、使 review 永远不可达。review 展示 exact org/current-window denominator，人工只能确认原样非空子集；确认事务把 selected rows 落为 `customer_provided/in`、其余 presented rows落为 `out`，再写 `operation_state.state_blob.active_recon_target_scope` 与 audit。该确认就是 active-scan boundary，成功后直接进 EAS，不再弹第二个 generic phase approval。typed fresh launch 的 `ConfirmedOrganizationIntake` 投影 `Some(false)`，历史 target 只有同 operation durable review 与当前 trusted snapshot exact-match 才能解锁；`ConfirmedTargetIntake` 投影 `Some(true)` 仍需读 DB truth。缺 org、无候选、空/编辑/新增/重复 response、Skip/timeout、candidate drift、DB error 均 fail closed并发出 `waiting_target_scope` + `ACTIVE_RECON_TRUSTED_TARGET_REQUIRED`。
- `stage_refiner` 是 gate/submit 失败后的唯一修复建议来源：gate 仍决定 PASS/BLOCK，StageRefiner 只查询/汇总 gate+DB-backed context，生成下一步 `RepairDirective`，不能伪造 StageDeliverable。EAS repair 必须 batch-first 且只暴露四个 `eas_*` wrapper，raw `pentest_run` / httpx / naabu / masscan / nmap / WhatWeb 都不进模型工具面。IP/CIDR 的 LIVENESS/PORT gap 收敛到 `eas_discover_ports`；CIDR 行到此为止，child IP 在 supplemental wave 才产生 SERVICE/WEB gap。concrete-IP SERVICE 只给 `eas_fingerprint_services`，并提示一次 targets batch、通常省略 ports：backend 自行按 IP pending ports 分片、并发、隔离慢目标和一次 bounded recovery，不能再让模型按相同端口集合分组或调大 timeout。exact-origin WEB 只给 `eas_fingerprint_web_stack`。submit 同时报 evidence_ref 和 coverage gap 时优先修 gap。Enumeration repair 从 DB worklist/exact roots 继续，不回到 URL 猜测。
- `stage_run` 的 request-scoped retry breaker 也是 orchestrator 控制流，不只是工具返回提示：`AgentExecutor::stage_run_retry_budget_exhausted` 把共享 guard 状态投影进 `execute_single_subtask`，text-only 与 gate-BLOCK 两条 reflector 分支都只在 breaker 未耗尽时自动重启 executor。耗尽后仍消费/记录最终 deterministic BLOCK，但本顶层请求立即停；下一次显式用户请求通过新的 `TopLevelRequestLease` 初始化 Task 后重置预算。
- Stage-scoped `SubtaskCompleted` 是 Gate PASS 语义，不是“本轮 agent loop 返回”语义：`run_stage_subtasks` 必须先读取 `StageFlowOutcome`，仅在 `gate_allowed=true` 时发完成事件并追加 `completed_results`。BLOCK 只进入 resumable pause，不能让前端显示 “Step complete”；generic pause 文案也不得无条件承诺再发一条消息即可越过 operator recovery。
- `RepairDirective.actions` 是 tool guard/checkpoint 的全量结构化真相，不能为了省 prompt 而截断；`model_instruction()` 是独立有界投影：写全量 `total + stable hash`、只展示原序前 20 条、明确用 `stage_worklist_next` 分页，并受 32 KiB UTF-8 硬上限约束。`to_submit_repair_mode()` 可携带这段已投影 directive，但下层不得再次展开同一 action list。
- StageRefiner 的 coverage-gap repair 先消费 `CoverageGapAction.suggested_capabilities`，缺失时再用 `stage_capability` registry 按 stage+technique 回填；`suggested_tools` 只作为兼容/具体实现 hint，并会把 EAS 的 raw `httpx/naabu/masscan/nmap/whatweb` hint 映射到 `eas_*` wrapper。SERVICE-FINGERPRINT 应收敛到 `eas.fingerprint_services`，WEB-FINGERPRINT 应收敛到 `eas.fingerprint_web_stack`，不能因为自然语言 repair reason 又放大成 broad service sweep。
- StageRefiner 对 `vuln_probe_anonymous_access` 的 command hint 必须让 worker 把 work item 的 exact `target_url` 同时作为 `query_target_data.exact_origin` 与 wrapper `target_url`；这样 planning 与 send-time authority 使用同一 origin，不会因 target row 仅保存裸域名而生成空 review set。
- EAS `asset_wave_barrier` 用当前 durable wave target ids/values 冻结轴；补波候选是
  该 operation/org/stage 从未进任何 wave 的 target，**不用** parent `started_at` 时间地板，
  所以首波 limit backlog 和运行中新资产都不会漏。CIDR child IP 即走该补波。
- Task-mode EAS close 不能读取 chat-session 全局 audit facts：只读 producer-org + current-target-owner + stage-fresh 的 guarded facts，并与同 org/run fresh `technique_outcomes`、fresh business Found 做交集；缺 operation `stage_started_at` 时不允许 presence fallback。该合同与 per-org `org_gate`、submit preview、StageAssetCoverage 相同。
- Task-mode stage close 按 stage 注入 `web_capable_assets`：Enumeration 只消费 EAS 确认的
  canonical exact origin，CIDR/wildcard 本行不能进 Web 分母；EAS 的 WEB 也只覆盖已
  确认 HTTP surface。缺省/查询失败 fail closed，不能让 stage-close/org_gate/UI 漂移。
- Enumeration close gate 必须从成功的 coverage snapshot 保留 exact-origin 轴：snapshot 明确 `assets: []` 时向 `GateContextBuilder::authoritative_in_scope_assets` 传 `Some([])`，与读模型一样 vacuous PASS；snapshot 缺失/查询失败则保持 `None` 并 fail-closed 回到自报轴。Enumeration 的 expected techniques 始终是 JS/DIR/PARAM/JSAPI 四轴，且只有存在 `stage_started_at` freshness cutoff 才可读取 `technique_outcomes`。
- Task-mode stage close读取 coverage 时必须透传当前 operation UUID；这不仅约束 completion freshness，也让 Enumeration 只消费该 operation 的 EAS transport handoff。无 operation 的 legacy/test provider可保守保留全部 exact origins，但生产 close不得用 session/latest marker替代 operation identity。
- `prompts::stage_charter` 必须告诉 agent：domain/url 做 Host/SNI LIVENESS + exact-origin
  WEB，IP 先 PORT 再 per-open-port SERVICE，CIDR 只 range LIVENESS/PORT 并由 child IP
  下波，wildcard 不执行。同 IP 多 Host/SNI 时，内部 nmap SERVICE 以 IP:port 为
  单位，内部 WhatWeb WEB 以 `scheme://host:port` 为单位；模型只调 wrapper。
- `prompts::stage_charter` 的 submit 示例不要把内部 canonical empty arrays 伪装成模型必填字段：`submit_stage_deliverable` 由后端补齐省略的 `evidence_refs` / `findings` / `coverage` / `skipped_checks` / `required_checks_done`；各阶段都不要求模型手写 evidence ids，若模型写了 id 则必须是真实 ledger id。普通 scope exclusion 不应被描述成 skipped check。
- 带 `StageSpec.specialist` 的阶段（如 `target_intel` → `recon`）必须让 primary 通过 `stage_run` 扇出；`synthesize_stage_subtask` / prompt 不应再要求 primary 直接调用 `recon_*`，否则会绕过 per-org worker gate。
- specialist fan-out 的阶段级 closeout 是 DB-authoritative aggregate gate，不是另一个 per-unit worker deliverable：`try_specialist_stage_gate` 解析 server-normalized pass-token 后，按 current operation、active-stage freshness floor、完整 engagement org subtree 与 TTL 重算 `org_stage_completions`；Investigation是唯一特例，其denominator和exact `(organization_id,passed_at)` 必须来自 `InvestigationAssetQueueRepository::load_resolution_closure` + `exact_completion_authority`，不再读取历史operation-global closure publication，也没有fallback。该路径不再调用 `enforce_trusted_submission` 要求 unit-less coordinator 伪造 durable unit row。非 specialist stage 仍必须通过 immutable captured submission，wrong/stale/foreign token 或缺 org completion 仍 fail closed。
- `TaskOrchestrator::set_force_stage_run_on_resume_once` 只用于裸 continuation resume：若当前 active stage 有 effective specialist（静态 `StageSpec.specialist`，或 persisted 双 `v2_only` 契约动态选择的 `attack_analyst` / `candidate_verifier`）且已绑定 `engagement_root`，下一次 primary loop 会通过 `ExecutionContext.harness_forced_tool="stage_run"` 强制先 dispatch `stage_run`；非 specialist/rootless/带 steering 的 resume 保持普通 agentic loop。该 hint 是 one-shot，进入 loop 后必须清掉，避免后续聊天继承。
- `TaskOrchestrator::resume(task_id, user_message, ...)` 把本次非空 continuation/steering 文本作为 request-local `ExecutionContext.task_input`，继而由 bridge 传到 `SubAgentContext.original_request` 和 stage-run worker 的 operator-constraint 摘录；不覆写 `tasks.input` 中的 durable 初始目标。空或全空白 continuation 明确回退 durable 初始目标。worker chain/checkpoint 和 request-scoped reentry guard 不参与该选择。
- Graph-flow 分支路由的 `made_progress` 必须按 stage 语义判断：漏洞阶段可以用 `findings_count`，但 `findings_allowed=false` 的信息收集/覆盖矩阵阶段（`target_intel` / `external_attack_surface` / `enumeration`）不能要求 findings；它们通过 DB/ledger truth、coverage handoff、stage_run pass token 等信号表示“有进展”，不要依赖模型提交的 evidence ids。否则 EAS 会因 `findings=[]` 被误判为无进展并走 `reporting` 短路，跳过 `enumeration`。
- RuntimeSupervisor 是运行中策略纠偏机制：只在 `ExecutionMonitor` 触发后运行，模型输出必须先解析成 `StrategyDirective` 并经过 stage/tool policy 裁剪，不能覆盖 gate，也不能覆盖 StageRefiner 的 post-gate `RepairDirective`。repair/stage_run 的确定性补洞仍以 StageRefiner 为准。

## Stage fork入口（2026-07-18）

- `TaskOrchestrator::stage_fork` 只携带 typed `StageForkCreate` 到 shared operation-create；stage执行仍走 `run_stage -> run_from_stage`，没有 CLI专用 executor、Gate或Company Controller。
- fork launch把当前数据库 Target快照作为 fresh invocation authority，但数据库仍验证快照非空/未漂移；普通 company-only fresh launch的 `Some(false)` 语义不变。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-kit task_orchestrator
```

## Hypothesis Analysis runtime contract（Plan B，2026-07-30）

- `hypothesis_analysis` 定义Controller/Analyst/Critic closed input/output schemas、bounded payload digest、server binding、artifact receipt与runtime/repository/runner ports。它不持有数据库、provider client、Gate或Plan C authority。
- runtime顺序固定为Controller dispatch → rolling Analyst → H1 seal → phased Critic map/reduce → H2/review → final Controller；small input可用1 lane，其余保持2–8个live lane，8是live concurrency而非lifetime item cap。Stage Team只记录control-plane与artifact receipt，不能把receipt/agent prose当truth。
- runtime末态 `AnalysisArtifactsReady` 只表示两波artifact与Controller final receipt已持久化；它不是stage PASS。production finalizer已安装，后续必须完成同一RR pre-Gate material → pure Gate → apply事务内compiler seal/canonical generation/outbox，得到`CandidateGenerationSealOutcome`后才可closeout/advance。
- semantic summary必须携带exact covered input/checklist、observed H1 proposal、typed missed checklist、blocker codes与bounded observations；Plan C/D终态、Campaign、Prepared Action和promotion均不在此模块。
