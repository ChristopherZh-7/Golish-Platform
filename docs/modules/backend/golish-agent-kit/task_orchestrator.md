# golish-agent-kit / task_orchestrator

> **一句话职责**：harness 驱动的自动化任务执行——一个 Task 由 metalcraft Executor 在 profile 投影的 Operation DAG 上推进：每阶段自规划+派发 specialist、提交 StageDeliverable、过确定性 evidence gate 才前进（大阶段边界 HITL），末尾 reporter 收尾。

- **类型**：目录模块（属于 crate [`golish-agent-kit`](../golish-agent-kit.md)）
- **路径**：`backend/crates/golish-agent-kit/src/task_orchestrator/`
- **状态**：✅ 已写卡

---

## 何时该读这张卡（给 AI 的触发提示）

- 改 Task 模式编排（stage 自规划/派发/gate/前进、reporter 收尾）时
- 改 `AgentExecutor` trait、`TaskOrchestrator::run` 入口、per-stage agentic loop 时
- 改 operation/task 断线恢复、runtime correction/refiner checkpoint、`operation_state.state_blob` schema 时

## 职责

在 `AgentBridge` 之上编排整个 Task 生命周期 + DB 持久化，每次 agent 调用回落到 bridge。`subtask_phases` 跑 Executor 驱动的 operation loop + `execute_single_subtask`（per-stage loop + gate）。

## 公开接口

| 符号 | 说明 |
|---|---|
| `TaskOrchestrator`（`run`） | 编排主体 + 入口 + 事件发射；fresh run 接 trusted `ProjectScopeRegistration`，并经 `RuntimeMemoryRepository` 原子创建 task/operation |
| `TaskOrchestrator::set_cli_runtime_scope` | headless fresh-only trusted seam；把一次解析的 root/descendants/threshold 交给 compound create，`run_from_stage` 消费后清空，resume 永不重建 scope |
| `AgentExecutor`（trait） | 每次 agent 调用的抽象（bridge 实现在 `golish-agent-bridge::bridge_executor`） |
| `agent_run_checkpoint` | P2a 细粒度 agent-run checkpoint DTO + `state_blob.agent_run` merge helpers |
| `stage_refiner` | Stage-aware deterministic repair owner：submit/gate 失败后生成 capability-first `RepairDirective`，并转换为 sub-agent `SubmitRepairMode` |
| `runtime_supervisor` | PentAGI-style in-run strategy supervisor：重复/停滞工具触发后解析 LLM JSON，按 stage/tool policy 裁剪成 `StrategyDirective` |
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

## 依赖

- crate 内 `harness`（gate/DAG）、`AgentBridge`（经 trait）；`golish-events`

## 注意事项 / 坑

- **不变量 I7/I8**：每阶段必须过 evidence gate 才前进；gate 是确定性规则，不能拿「agent 自信说完成」当通过。
- Verification flow 必须同时读取 operation persisted `RuntimeMemoryContract` 与 `AttackExecutionContract`：只有两者都为 `v2_only` 才进入 Candidate verifier/exact DB truth；同一 effective-specialist 解析也用于 fresh stage prompt 与裸 resume 的 `stage_run` one-shot force，使 V2 Verification 动态得到 `candidate_verifier`，而 Legacy/dual Verification 不会因 additive worker 获得 specialist。contract lookup 失败时 Candidate primary 留在 specialist-only 路由，交由 `stage_run` 的权威重读 fail closed。Legacy/dual 保留旧 deliverable candidate 的 bounded chain-wave fallback。V2 必须取得 server-owned authority envelope 声明的同 operation/scope/current-wave 全部 unit snapshot，missing/extra/DB error/foreign/inconsistent 一律 BLOCK，不能从返回行自推 scope 或回退进程内 wave state。
- Reporting stage entry 先通过 `DbRepoProvider::reporting_build_validated_revision` 从完整 canonical DB source set build/reuse validated revision；失败时在任何 agent turn 前 BLOCK。stage close 再用 `reporting_gate_truth` 重读 current truth。Reporting 跳过 generic enrichment/planner/wiki prior，只允许最小 StageDeliverable；stage seam 不持有 artifact/finalizer 能力，最终发布只能走显式本地 operator command。
- `bridge_executor`（`AgentExecutor` 实现）在 `golish-agent-bridge`（依赖 AgentBridge）；本模块只持 trait。
- graph-flow 的 `operation_state.current_stage` 表示**当前正在执行的 stage**：进入新 stage 时同步并刷新 `stage_started_at`；断线后回到同一 stage 时不能重复刷新，否则 freshness-window gate 会看不到断线前已落库的 evidence。
- `operation_state.state_blob` 是 graph checkpoint、stage_run worker resume 等多消费者共享 JSON；更新 `graph_flow` 时要 merge 保留其他 key，不能整段覆盖。
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
- `stage_refiner` 是 gate/submit 失败后的唯一修复建议来源：gate 仍决定 PASS/BLOCK，StageRefiner 只查询/汇总 gate+DB-backed context，生成下一步 `RepairDirective`，不能伪造 StageDeliverable。EAS repair 必须 batch-first 且只暴露四个 `eas_*` wrapper，raw `pentest_run` / httpx / naabu / masscan / nmap / WhatWeb 都不进模型工具面。IP/CIDR 的 LIVENESS/PORT gap 收敛到 `eas_discover_ports`；CIDR 行到此为止，child IP 在 supplemental wave 才产生 SERVICE/WEB gap。concrete-IP SERVICE 只给 `eas_fingerprint_services`，exact-origin WEB 只给 `eas_fingerprint_web_stack`。submit 同时报 evidence_ref 和 coverage gap 时优先修 gap。Enumeration repair 从 DB worklist/exact roots 继续，不回到 URL 猜测。
- `stage_run` 的 request-scoped retry breaker 也是 orchestrator 控制流，不只是工具返回提示：`AgentExecutor::stage_run_retry_budget_exhausted` 把共享 guard 状态投影进 `execute_single_subtask`，text-only 与 gate-BLOCK 两条 reflector 分支都只在 breaker 未耗尽时自动重启 executor。耗尽后仍消费/记录最终 deterministic BLOCK，但本顶层请求立即停；下一次显式用户请求通过新的 `TopLevelRequestLease` 初始化 Task 后重置预算。
- `RepairDirective.actions` 是 tool guard/checkpoint 的全量结构化真相，不能为了省 prompt 而截断；`model_instruction()` 是独立有界投影：写全量 `total + stable hash`、只展示原序前 20 条、明确用 `stage_worklist_next` 分页，并受 32 KiB UTF-8 硬上限约束。`to_submit_repair_mode()` 可携带这段已投影 directive，但下层不得再次展开同一 action list。
- StageRefiner 的 coverage-gap repair 先消费 `CoverageGapAction.suggested_capabilities`，缺失时再用 `stage_capability` registry 按 stage+technique 回填；`suggested_tools` 只作为兼容/具体实现 hint，并会把 EAS 的 raw `httpx/naabu/masscan/nmap/whatweb` hint 映射到 `eas_*` wrapper。SERVICE-FINGERPRINT 应收敛到 `eas.fingerprint_services`，WEB-FINGERPRINT 应收敛到 `eas.fingerprint_web_stack`，不能因为自然语言 repair reason 又放大成 broad service sweep。
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
- `TaskOrchestrator::set_force_stage_run_on_resume_once` 只用于裸 continuation resume：若当前 active stage 有 effective specialist（静态 `StageSpec.specialist`，或 persisted 双 `v2_only` 契约动态选择的 `attack_analyst` / `candidate_verifier`）且已绑定 `engagement_root`，下一次 primary loop 会通过 `ExecutionContext.harness_forced_tool="stage_run"` 强制先 dispatch `stage_run`；非 specialist/rootless/带 steering 的 resume 保持普通 agentic loop。该 hint 是 one-shot，进入 loop 后必须清掉，避免后续聊天继承。
- `TaskOrchestrator::resume(task_id, user_message, ...)` 把本次非空 continuation/steering 文本作为 request-local `ExecutionContext.task_input`，继而由 bridge 传到 `SubAgentContext.original_request` 和 stage-run worker 的 operator-constraint 摘录；不覆写 `tasks.input` 中的 durable 初始目标。空或全空白 continuation 明确回退 durable 初始目标。worker chain/checkpoint 和 request-scoped reentry guard 不参与该选择。
- Graph-flow 分支路由的 `made_progress` 必须按 stage 语义判断：漏洞阶段可以用 `findings_count`，但 `findings_allowed=false` 的信息收集/覆盖矩阵阶段（`target_intel` / `external_attack_surface` / `enumeration`）不能要求 findings；它们通过 DB/ledger truth、coverage handoff、stage_run pass token 等信号表示“有进展”，不要依赖模型提交的 evidence ids。否则 EAS 会因 `findings=[]` 被误判为无进展并走 `reporting` 短路，跳过 `enumeration`。
- RuntimeSupervisor 是运行中策略纠偏机制：只在 `ExecutionMonitor` 触发后运行，模型输出必须先解析成 `StrategyDirective` 并经过 stage/tool policy 裁剪，不能覆盖 gate，也不能覆盖 StageRefiner 的 post-gate `RepairDirective`。repair/stage_run 的确定性补洞仍以 StageRefiner 为准。

## 测试入口

```bash
cd backend && cargo nextest run -p golish-agent-kit task_orchestrator
```
