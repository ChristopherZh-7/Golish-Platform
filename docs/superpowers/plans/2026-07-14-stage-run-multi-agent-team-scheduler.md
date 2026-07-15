# Stage Run Durable Multi-Agent Team Scheduler 实现计划

> Superseded by `docs/superpowers/plans/2026-07-15-stage-run-company-controller-agent.md` for
> the `target_intel` V2 orchestration shape. The durable scheduler foundation remains in use.

> **执行状态（2026-07-14）**：Tasks 1–8 的 durable schema、RuntimeMemory、sibling queue、
> barrier、唯一 Aggregator、read model/UI 与 `target_intel` pilot 已在工作树实现；startup
> recovery、operator exact-CAS resolution 与 Gate BLOCK→repair generation→fresh Aggregator
> continuation 已完成聚焦验证。
> Task 9 的 EAS/Enumeration/Candidate rollout 仍按设计留在 gate 后，Verification 不纳入通用
> Team Scheduler。Task 10 的 `init.sh` / `just precommit` / live acceptance 依用户本轮指令延后，
> 不得据此把 feature 标为 `passing`。

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:test-driven-development 与
> superpowers:executing-plans 逐任务实现此计划。

**目标：** 把 `stage_run` 从每组织一个串行 Worker 改成服务端持久化、可排队、可恢复、可受控
并发的 sibling multi-agent team，同时保持 Unit/Gate/evidence/final-seal 的确定性边界。

**架构：** 一个 Main Agent 只负责控制面。每个 StageRunUnit 拥有 immutable TeamPlan；
WorkItem 是排队单位，每次执行创建独立 WorkerRun/lease/message chain。producer/helper 只提交
immutable WorkerOutput，唯一 Aggregator 在 manifest epoch 关闭且 sibling barrier 满足后提交
deliverable，并在同一事务关闭 Aggregator、Unit 与 handoff。bound Worker 继续禁止嵌套 agent；
动态协作通过 durable WorkerRequest 创建 sibling WorkItem。

**技术栈：** Rust 2021、sqlx/PostgreSQL、rig agent runtime、Tauri 2、ts-rs、React 19、
TypeScript 6、Vitest。

---

### 任务 1：锁定 single-worker compatibility 与错误提前 finalization

**文件：**

- 修改：`backend/crates/golish-db/tests/runtime_memory_worker_transactions.rs`
- 修改：`backend/crates/golish-sub-agents/src/executor/response_parsing.rs`

**步骤：**

1. 增加 team-of-one seed/replay exact-id 测试。
2. 增加两个 WorkItem 获得不同 Worker/chain/lease 的 RED 测试。
3. 增加 producer complete 后 Unit 仍 Running、live/retry sibling存在时 finalizer拒绝、只有
   Aggregator可 finalize 的 RED 测试。
4. 保留并锁定 `BOUND_WORKER_NESTED_DELEGATION_BLOCKED`，证明实现不会靠放开嵌套调用完成。

### 任务 2：安装 Team Scheduler additive schema

**文件：**

- 新增：`backend/crates/golish-db/migrations/20260714000003_stage_team_scheduler.sql`
- 修改：`backend/crates/golish-db/tests/runtime_memory_worker_transactions.rs`
- 修改：`backend/crates/golish-db/tests/runtime_memory_rollout_migrations.rs`

**步骤：**

1. 新建 `stage_team_plans`：unit/version/hash、role allowlist、aggregator kind/role、worker limits、
   dynamic request policy、dispatch epoch、requests_closed_at 与 final submitter。
2. 新建 `stage_work_items`、`stage_work_item_dependencies`、`stage_worker_outputs`、
   `stage_worker_requests`；给 `stage_worker_runs` 增 nullable `work_item_id` 兼容历史。
3. 加 plan/unit唯一、stable work key、one-live-worker-per-item、terminal Worker唯一 immutable output、
   request dedupe与 closed-epoch约束。
4. migration trigger 确保 team contract 下 Worker必须绑定 item，各 owner tuple不能跨 operation、
   snapshot、Unit、organization。

### 任务 3：扩展 TeamPlan/WorkItem/Output/Request domain 与 repository trait

**文件：**

- 修改：`backend/crates/golish-agent-kit/src/db_traits/runtime_memory.rs`
- 修改：`backend/crates/golish-agent-kit/src/harness/stage_spec.rs`
- 修改：`backend/crates/golish-agent-kit/src/harness/resources.rs`
- 修改：`backend/crates/golish-agent-kit/src/db_traits/repo.rs`

**步骤：**

1. 新增 frozen plan、queue item、dependency、output、request、barrier 与 team read-model DTO。
2. 把 `SeedStageRuntime` 扩成 team-aware seed，team-of-one仍返回旧 Unit/primary Worker identity。
3. 新增 `claim_stage_work_item`、`complete_stage_worker`、`request_stage_worker`、
   `close_stage_request_epoch`、`load_stage_team_barrier`、`claim_stage_aggregator` 与
   `finalize_stage_team_unit` trait。
4. execution status 与 business output 分离：retryable failure回 queue；producer可 execution PASS
   且 output business blocked；`gate_blocked` 仅用于 Aggregator/Unit。

### 任务 4：实现 team-of-one 兼容 seed 与 immutable Output

**文件：**

- 修改：`backend/crates/golish-db/src/repo/runtime_memory_tx.rs`
- 修改：`backend/crates/golish-db/src/repo/stage_run_units.rs`
- 修改：`backend/crates/golish-db/src/repo/stage_worker_runs.rs`
- 新增：`backend/crates/golish-db/src/repo/stage_teams.rs`
- 修改：`backend/crates/golish-db/src/repo/mod.rs`
- 修改：`backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs`

**步骤：**

1. V2 seed 原子创建 Unit、TeamPlan、stable primary WorkItem 与 primary Worker；重复调用返回同一
   identity/hash，不重复分配 chain/lease。
2. 新 `complete_stage_worker` 只能结束 non-aggregator WorkItem，并要求 accepted immutable
   WorkerOutput；它不修改 Unit/handoff。
3. 旧 single-worker final seal 继续服务 legacy/dual与兼容 team-of-one，禁止把公共
   `finish_attempt_cas(Passed)`放宽成任意 producer可关闭 Unit。
4. 先让所有新表在 K=1 路径产生真实数据，再开启 sibling。

### 任务 5：实现 request epoch、sibling barrier 与唯一 Aggregator finalizer

**文件：**

- 修改：`backend/crates/golish-db/src/repo/stage_teams.rs`
- 修改：`backend/crates/golish-db/src/repo/runtime_memory_tx.rs`
- 修改：`backend/crates/golish-db/src/repo/stage_handoffs.rs`
- 修改：`backend/crates/golish-db/tests/runtime_memory_worker_transactions.rs`

**步骤：**

1. WorkerRequest 在 open epoch 内按 `(plan, epoch, parent WorkItem, dedupe_key, semantic payload)`
   exact replay；语义 hash 排除 WorkerRun/lease/attempt/checkpoint 等瞬时 fence，但每次 replay
   返回前仍验证当前 exact live fence。接受 request 与创建 sibling WorkItem在同一事务。
2. Aggregator claim 前原子关闭 epoch；关闭后新 request稳定拒绝。
3. barrier 重算 required WorkItem全部 terminal、依赖满足、无 live lease/tool、无 retry/recovery、
   每个 terminal producer有 immutable Output，且 manifest hash未漂移。
4. `finalize_stage_team_unit` 只允许唯一 Aggregator/final submitter，在同一事务关闭 Aggregator、
   Unit、handoff/completion/final seal；response-loss exact replay不重复副作用。
5. Gate BLOCK 冻结旧 epoch/aggregator并创建 repair generation，不复活 terminal Worker。

### 任务 6：改造 resume、startup reaper 与 legacy mirror

**文件：**

- 修改：`backend/crates/golish/src/stage_run/runtime_v2.rs`
- 修改：`backend/crates/golish-db/src/repo/tasks.rs`
- 修改：`backend/crates/golish-db/src/repo/message_chains.rs`
- 修改：`backend/crates/golish-db/src/repo/runtime_memory_reaper.rs`

**步骤：**

1. 移除 `workers == units` 与 `unit_workers.len()==1` 假设，按 TeamPlan/WorkItem完整性恢复。
2. reaper 按 exact WorkItem/Worker owner 与 lease处理，不再用 specialist等于Unit specialist连接。
3. expired producer依据 active-tool checkpoint决定 retry_pending/recovery_required；Aggregator lease过期
   可被同 generation server re-claim，但 final submitter CAS保持唯一。
4. DualWrite固定 team-of-one；多 Worker只在 V2Only启用，避免 legacy org-level mirror被 sibling覆盖。

### 任务 7：实现 durable WorkerRequest sibling 工具

**文件：**

- 修改：`backend/crates/golish-agent-kit/src/harness/tool_taxonomy.rs`
- 修改：`backend/crates/golish-agent-kit/src/harness/stage_capability.rs`
- 修改：`backend/crates/golish-agent-app/src/ai/commands/bridge_config.rs`
- 新增：`backend/crates/golish-agent-app/src/ai/stage_worker_request_tool.rs`
- 修改：`backend/crates/golish-agent-app/src/ai/mod.rs`
- 修改：`backend/crates/golish-sub-agents/src/defaults/builder/registry.rs`
- 修改：`backend/crates/golish-sub-agents/src/executor_types.rs`
- 修改：`backend/crates/golish-sub-agents/src/executor/response_parsing.rs`

**步骤：**

1. 工具参数只含 requested role/kind、subject refs、reason、output schema、budget hint、dedupe key；
   operation/stage/unit/plan/item/worker/epoch全部由 bound context注入。
2. 服务端校验 role allowlist、scope、max workers、budget、dependency与 epoch，事务内记 request decision。
3. accepted request只入 durable queue，不在当前 Worker栈内直接创建/运行 agent。
4. bound nested sub-agent guard保持原样；增加 regression证明 request tool可用而 `sub_agent_*`仍拒绝。

### 任务 8：把 stage_run 改成 durable queue drain

**文件：**

- 修改：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`
- 修改：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs`
- 修改：`backend/crates/golish-agent-runtime/src/execution_mode/prompt_render.rs`
- 修改：`backend/crates/golish/src/stage_run/fleet.rs`
- 修改：`backend/crates/golish/src/stage_run/mod.rs`

**步骤：**

1. scheduler从 durable WorkItem claim，不再把 transient org循环当队列真值；首版 K=1 drain完整跑通。
2. producer/helper按 frozen role prompt执行并写 WorkerOutput；依赖未完成时 durable waiting，不占 live agent。
3. queue空且 epoch可关闭时 claim Aggregator，聚合 immutable outputs并走唯一 finalizer。
4. 增加 bounded global/per-operation/per-org limits、round-robin公平、priority aging与 risk lane；默认
   target_intel pilot允许 K>1，active/exploit stage保持 K=1。
5. Verification继续使用 CandidateAttempt scheduler，不继承通用 Team并发。

### 任务 9：定义静态分片策略与 dynamic request policy

**文件：**

- 修改：`backend/crates/golish-agent-kit/src/harness/stage_spec.rs`
- 修改：`resources/harness/stages/target_intel/spec.json`
- 修改：`resources/harness/stages/external_attack_surface/spec.json`
- 修改：`resources/harness/stages/enumeration/spec.json`
- 修改：`resources/harness/stages/attack_candidate/spec.json`

**步骤：**

1. spec声明 allowed roles、work item kinds、static sharding、aggregator role、max workers、dynamic request
   policy、output schema与risk lane。
2. target_intel先按 provider/asset class静态分片并开启安全的 dynamic helper request。
3. EAS/Enumeration按 exact target/origin/axis 分片，保持 wrapper/authorization边界；active stage K=1
   验证后再逐步提高。
4. attack_candidate允许 evidence analyst/coverage critic/hypothesis synthesizer sibling，但只有
   Aggregator可写 candidate/no_candidate decision；Verification spec明确不采用该模式。

### 任务 10：补 DB-backed Team read model、UI 与 run_tree

**文件：**

- 修改：`backend/crates/golish-agent-app/src/ai/commands/stage.rs`
- 修改：`backend/crates/golish-core/src/events/harness_trace.rs`
- 修改：`backend/crates/golish/src/commands_facade/ai.rs`
- 修改：`backend/crates/golish/src/commands_registry.rs`
- 新增：`frontend/lib/api/stage-team.ts`
- 新增：`frontend/components/Engagement/StageTeamRunView.tsx`
- 修改：`frontend/components/Engagement/StageRunOrgRows.tsx`
- 修改：`frontend/components/ToolCallDetailView/ToolCallDetailView.tsx`
- 修改：`frontend/services/ai-events/harness-handlers.ts`
- 修改：`frontend/store/types/session.ts`
- 修改：`scripts/run_tree.py`

**步骤：**

1. 新增 exact `(operation_id, stage_execution_id)` Team read command；返回
   Unit→WorkItem→Worker→Output/Request/Barrier/Gate，隐藏 lease token、raw checkpoint与 secret。
2. harness trace只携 stage_execution_id/unit refresh pointer，前端从 DB刷新，不把 event snapshot当真值。
3. UI显示排队、依赖、当前角色、重试/recovery、business output、Aggregator barrier与终态 Gate。
4. `run_tree.py` 加 TeamPlan/WorkItem/Output/Request/epoch/barrier树，保留旧 run兼容。

### 任务 11：更新模块卡并做聚焦验收

**文件：**

- 修改：`docs/modules/backend/golish-db.md`
- 修改：`docs/modules/backend/golish-db/repo.md`
- 修改：`docs/modules/backend/golish-agent-kit/db_traits.md`
- 修改：`docs/modules/backend/golish-agent-kit/harness.md`
- 修改：`docs/modules/backend/golish-agent-runtime/agentic_loop.md`
- 修改：`docs/modules/backend/golish-sub-agents/executor.md`
- 修改：`docs/modules/backend/golish/stage_run.md`
- 修改：`docs/modules/backend/golish-agent-app/ai.md`
- 修改：`docs/modules/frontend/components.md`
- 修改：`docs/modules/frontend/lib.md`
- 修改：`docs/modules/frontend/services.md`
- 修改：`docs/modules/frontend/store.md`
- 修改：`docs/modules/INDEX.md`

**步骤：**

1. 同步职责、公开接口、依赖、迁移、不变量与精确测试入口。
2. 逐条运行新增 DB/runtime/sub-agent/frontend 测试，先 K=1，再 target_intel V2Only K>1 fixture。
3. 运行相关 Vitest、typecheck、rustfmt、JSON 与 scoped diff check。
4. 按用户本轮指令不运行 `init.sh`、`just precommit` 或全量 suite；这些未跑之前 feature保持
   `in_progress`，不能宣称完成。
