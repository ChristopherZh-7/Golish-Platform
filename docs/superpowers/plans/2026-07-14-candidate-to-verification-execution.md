# Candidate 到 Verification 可恢复执行实现计划

> **执行状态（2026-07-14）**：TerminalIntent/barrier/terminalizer、operator recovery、
> approval start-before、versioned recipe、same-Attempt submit-only、typed FactDelta direct route、
> refuted no-attack 与 pending enrichment queue 已在工作树实现。信息不足的 enrichment 当前是
> 可观察、不可变、fail-closed 的待处理 authority，不宣称已有自动 enrichment executor。
> TerminalIntent/recovery、四类 FactDelta route、ts-rs queue projection 与相关 UI 均已完成聚焦
> 测试；本计划的代码切片不再处于“正在实现”状态。
> `init.sh` / `just precommit` / live model acceptance 按用户本轮指令延后，feature 保持
> `in_progress`。

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:test-driven-development 与
> superpowers:executing-plans 逐任务实现此计划。

**目标：** 把已审批 Candidate 的逐条验证改造成可从任意崩溃点恢复、执行合同不可静默漂移、
审批过期可重新决策、结果可由确定性 terminalizer 原子收口的 CandidateAttempt 协议。

**架构：** 模型只提交 immutable TerminalIntent；服务端在 active-tool 清空并持久化 exact
checkpoint barrier 后消费 intent。Attempt、Finding/lineage、FactDelta、Worker/lane 与 outbox
仍由单一短事务终结。`outcome_unknown` 只能通过三种受限 operator CAS 处置；V2 recipe 必须
命中版本化 typed adapter，generic runner 不再作为 V2 fallback。

**技术栈：** Rust 2021、sqlx/PostgreSQL、Tauri 2、ts-rs、React 19、TypeScript 6、Vitest。

---

### 任务 1：锁定四个崩溃窗口与审批过期行为

**文件：**

- 修改：`backend/crates/golish-db/tests/attack_execution_v2_migrations.rs`
- 修改：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`

**步骤：**

1. 增加精确测试，分别构造 intent commit 后、tool finish 后、checkpoint 后、terminalizer commit
   response-loss 后的状态；断言外部 action 只执行一次，且每种状态都能确定性恢复。
2. 增加 expiry 测试：未开始 action 的 Attempt 可 abandon 并重新打开 review；已 started、
   completed 或 submitted 的 Attempt 不能 abandon，只能 finish/recovery。
3. 增加 outcome-unknown 测试，锁定仅三种 operator decision，拒绝修改 target、plan、args、
   budget、evidence owner。
4. 先运行每条新增测试并记录 RED 原因，不运行整个 crate。

**定向命令：**

```bash
cd backend
cargo test -p golish-db --test attack_execution_v2_migrations candidate_terminal_intent -- --nocapture
cargo test -p golish-db --test attack_execution_v2_migrations candidate_approval_start_before -- --nocapture
```

### 任务 2：安装 Candidate recovery additive schema

**文件：**

- 新增：`backend/crates/golish-db/migrations/20260714000002_candidate_verification_recovery.sql`
- 修改：`backend/crates/golish-db/tests/attack_execution_v2_migrations.rs`
- 修改：`backend/crates/golish-db/tests/attack_rollout_cohort_migrations.rs`

**步骤：**

1. 给 approval 增加 `start_before`，由旧 `expires_at` 回填；保留旧列兼容，但新 claim/action
   authority 只使用 `start_before`。
2. 给 action journal 增加 immutable authorization receipt：approval/decision version、plan/scope
   hash、authorized_at、start_before 与 receipt hash。
3. 新建 `candidate_attempt_terminal_intents`、`candidate_attempt_terminal_barriers`、
   `candidate_attempt_terminal_receipts`、`candidate_recovery_cases` 与 recovery evidence join。
4. 用 closed enum/check、composite FK、request-id 唯一键、immutable trigger 与 deferred authority
   约束 exact Attempt/Candidate/Worker/plan/result/checkpoint identity。
5. 增加 migration shape、immutability、cross-owner rejection 与 response-loss replay 测试。

### 任务 3：把 recipe 与 executor contract 纳入 immutable plan

**文件：**

- 修改：`backend/crates/golish-agent-kit/src/harness/attack_execution/types.rs`
- 修改：`backend/crates/golish-agent-kit/src/harness/attack_execution/classifier.rs`
- 修改：`backend/crates/golish-agent-kit/src/harness/attack_execution/decision.rs`
- 修改：`backend/crates/golish-agent-kit/src/harness/attack_execution/tests.rs`
- 修改：`backend/crates/golish-db/src/repo/attack_candidates.rs`
- 修改：`backend/crates/golish-db/src/repo/attack_candidate_approvals.rs`

**步骤：**

1. 在 `CandidateExecutionPlan` 与每个 action 增加 `recipe_version`、
   `executor_contract_version`、`subject_refs`、`preconditions`、`control_plan`、
   `expected_signal`、`conflict_key`。
2. canonical plan hash 覆盖上述字段；同一 plan hash 不得在部署后选择不同 executor。
3. approval snapshot/read model 显式保留版本；未开始的 V1 generic plan 回到 `proposed` 并新开
   decision version，已 started V1 仍按冻结旧合同收口。
4. 对 unknown V2 recipe/version fail closed，并增加纯领域 RED/GREEN 测试。

### 任务 4：实现 TerminalIntent、barrier 与 server-authority terminalizer

**文件：**

- 修改：`backend/crates/golish-agent-kit/src/db_traits/runtime_memory.rs`
- 修改：`backend/crates/golish-agent-kit/src/db_traits/repo.rs`
- 修改：`backend/crates/golish-db/src/repo/candidate_attempts.rs`
- 修改：`backend/crates/golish-db/src/repo/finding_lineage.rs`
- 修改：`backend/crates/golish-db/src/repo/runtime_memory_tx.rs`
- 修改：`backend/crates/golish-db/src/repo/mod.rs`
- 修改：`backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs`
- 修改：`backend/crates/golish-agent-app/src/ai/candidate_submit_tool.rs`

**步骤：**

1. `submit_candidate_attempt` 在有效 Worker/tool lease 下只写 immutable terminal intent，并返回
   deterministic ToolResult；Attempt 派生为 `terminalization_pending`，不释放 lane/Worker。
2. `checkpoint_bound_worker_chain` 在同一事务校验 full ToolCall/ToolResult、active tool 已空，
   并写 exact terminal barrier；response-loss 重放返回同一 barrier。
3. 新 server-authority terminalizer 只消费 barrier-ready intent，不再依赖原 executor lease仍活着；
   它原子写 Attempt/Candidate terminal、Finding/lineage、FactDelta、receipt/outbox，最后释放
   Worker/lane。
4. intent 存在后禁止更多外部 action、新 Attempt 或 plan mutation；重复 terminalizer 返回同一
   terminal receipt。
5. 保留旧 operation compatibility seam；V2-only 禁止绕过 intent 直接 terminalize。

### 任务 5：实现过期与 outcome-unknown 的 operator CAS

**文件：**

- 修改：`backend/crates/golish-db/src/repo/attack_candidate_approvals.rs`
- 修改：`backend/crates/golish-db/src/repo/candidate_attempts.rs`
- 修改：`backend/crates/golish-db/src/repo/candidate_review_barriers.rs`
- 修改：`backend/crates/golish-agent-kit/src/db_traits/runtime_memory.rs`
- 修改：`backend/crates/golish-agent-app/src/ai/commands/attack.rs`

**步骤：**

1. claim/begin-action 分别校验 start-before；begin-action 原子写 authorization receipt。
2. expiry reaper 对“无 action started”Attempt 做 `abandoned`、释放 Worker/lane、Candidate→
   `proposed`，reopen review barrier，并递增 decision version；Attempt 仍计 attempts_total，但不计
   actions_started。
3. 建 recovery read model 与 request-id/row-version CAS；只接受
   `terminalize_blocked_outcome_unknown`、`abandon_before_side_effect`、
   `accept_external_result_with_exact_evidence`。
4. started/completed/submitted 路径拒绝 abandon/reopen，保留 exact evidence ownership。

### 任务 6：接入 verifier runtime 的 recover-before-claim 协议

**文件：**

- 修改：`backend/crates/golish-sub-agents/src/executor/response_parsing.rs`
- 修改：`backend/crates/golish-sub-agents/src/executor/inner.rs`
- 修改：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`
- 修改：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/candidate_verification.rs`
- 修改：`backend/crates/golish-agent-runtime/src/agentic_loop/single_tool_call.rs`

**步骤：**

1. 每次 scheduler tick 先 drain barrier-ready intents，再处理可重放 tool finish/checkpoint，最后
   才 claim 新 CandidateAttempt。
2. intent 后不再调外部工具；action 前 verifier 失败走安全 release/retry，action started 且结果
   未知创建 recovery case。
3. exact response-loss replay 不依赖原进程内存；submitted orphan 不再被 lane reaper误报为无 owner。
4. global exploit lane 保持并发 1；Verification 不接入通用 Team Scheduler 并发。

### 任务 7：建立 typed adapter registry 并隔离 generic V1

**文件：**

- 修改：`backend/crates/golish-pentest-app/src/pentest_bridge/verification_capabilities.rs`
- 修改：`backend/crates/golish-pentest-app/src/pentest_bridge/vuln_adapters/mod.rs`
- 新增：`backend/crates/golish-pentest-app/src/pentest_bridge/vuln_adapters/verification_recipes.rs`
- 修改：`backend/crates/golish-agent-kit/src/harness/attack_execution/classifier.rs`

**步骤：**

1. 定义 typed recipe registry：exact Nuclei replay、anonymous request replay、differential HTTP、
   parameter injection 与 configuration evidence evaluation。
2. adapter 输入只能来自 frozen subject refs/request template/identity slot；模型仍只传 ordinal。
3. V2 unsupported recipe 返回稳定 blocker，绝不进入 `execute_legacy_action`；legacy operation按旧
   contract 隔离保留。
4. proof/refutation/blocker evidence 必须由当前 action journal产生，禁止复用旧 evidence充当结果。

### 任务 8：修复 FactDelta follow-on 路由

**文件：**

- 修改：`backend/crates/golish-agent-kit/src/harness/attack_execution/types.rs`
- 修改：`backend/crates/golish-db/src/repo/attack_wave_consolidations.rs`
- 修改：`backend/crates/golish-db/src/repo/attack_candidate_work_items.rs`
- 修改：`backend/crates/golish-db/tests/attack_execution_v2_migrations.rs`

**步骤：**

1. 分开 `delta_kind`、`observation_kind`、`allowed_techniques` 与
   `enrichment_required`，`refuted` 默认不创建新攻击 work item。
2. consolidation 在 consume delta/open Wave 前验证每个 route 能形成 classifier 支持的 typed
   observation；recognized-but-unsupported route 整个事务回滚，delta 保持未消费。
3. 强 typed evidence 直接冻结 follow-on manifest 并原子打开下一 Wave；`refuted` 可 accepted，但
   只形成 `no_attack` member，不创建攻击 WorkItem。
4. 信息不足只写 immutable delta-local pending enrichment authority，稳定返回
   `pending_enrichment`；source Wave 不关闭、FactDelta 不消费、target Wave/WorkItem 不创建，禁止把
   `created/updated/new_surface` 当 technique，也不宣称当前版本会自动执行 enrichment。
5. orchestrator 将 `pending_enrichment` 作为可观察的显式 BLOCK 而非 invalid/error；Verification
   queue 返回 pending count、subject、reason 与 allowed techniques，不暴露 request/raw evidence。
6. 增加 pending replay/source-stability、supported direct route、no-attack refuted 与 unsupported
   rollback 测试。

### 任务 9：补 Verification queue/recovery IPC 与 UI

**文件：**

- 修改：`backend/crates/golish-agent-app/src/ai/commands/attack.rs`
- 修改：`backend/crates/golish/src/commands_facade/ai.rs`
- 修改：`backend/crates/golish/src/commands_registry.rs`
- 修改：`frontend/lib/api/attack.ts`
- 修改：`frontend/components/Engagement/AttackCandidateReview.tsx`
- 修改：`frontend/components/Engagement/CandidateAttemptRows.tsx`
- 修改：`frontend/components/ToolCallDetailView/ToolCallDetailView.tsx`
- 修改：对应 `.test.tsx` 文件

**步骤：**

1. 新增 `attack_list_verification_queue` 与 `attack_resolve_candidate_recovery`；exact
   operation/wave scope，DTO 隐藏 lease token、raw checkpoint 与 secret。
2. queue 返回 observation/evidence、recipe/executor version、approval start-before、Attempt、
   Worker、action journal、budget、intent/barrier、recovery case 与 Wave consolidation摘要。
3. UI 区分 queued/running/terminalization pending/recovery required/terminal，并只展示三种合法
   recovery action；所有 mutation 带 request id 与 expected versions。
4. 用 ts-rs 生成类型，禁止手改 `frontend/lib/generated/`。

### 任务 10：更新 stage contract、模块卡与聚焦验收

**文件：**

- 修改：`resources/harness/stages/verification/spec.json`
- 修改：`docs/modules/backend/golish-db.md`
- 修改：`docs/modules/backend/golish-db/repo.md`
- 修改：`docs/modules/backend/golish-agent-kit/harness.md`
- 修改：`docs/modules/backend/golish-agent-runtime/agentic_loop.md`
- 修改：`docs/modules/backend/golish-agent-app/ai.md`
- 修改：`docs/modules/backend/golish-pentest-app/pentest_bridge.md`
- 修改：`docs/modules/frontend/components.md`
- 修改：`docs/modules/frontend/lib.md`
- 修改：`docs/modules/INDEX.md`

**步骤：**

1. V2 dependency 明确从 `attack_candidate` 进入 Verification；legacy contract保持兼容路由。
2. 更新模块职责、公开接口、schema 与测试入口。
3. 只运行每个新测试的精确 filter、前端相关 Vitest、typecheck、rustfmt、JSON 与 scoped diff check。
4. 遵从用户本轮指令，不运行 `init.sh`、`just precommit` 或全量测试；把这些 completion gate保留
   在 feature `in_progress` 的后续验证清单里。
