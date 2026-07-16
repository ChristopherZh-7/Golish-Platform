# EAS reconciled child 与 CLI continuation 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 让 safe-reconciled EAS child 被真实 Company Controller scheduler 重新执行，并让 CLI 通过 GUI/CLI 共用的 durable Turn claim 接管同一 operation 完成实体验收。

**架构：** Controller 不可 claim 时先按 barrier 区分 operator recovery 与 runnable children；后者进入现有 child drain，再回到同 Controller loop。CLI 保留全部 exact identity/fence 校验，但改用 `golish-agent-app` 的 source + open-Turn CAS，不再维护独立 waiting-to-running claim。

**技术栈：** Rust 2021、sqlx/PostgreSQL、rig scripted model、cargo nextest、真实 `golish --stage-run-resume`、`scripts/run_tree.py --full --db`。

## 文件结构

- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`：leader-none 三态与 child redrain。
- `backend/crates/golish/src/stage_run/mod.rs`：exact selector、shared Turn claim 与 CLI resume orchestration。
- `backend/crates/golish-agent-app/src/ai/commands/core/operation_resume.rs`：导出 GUI/CLI 共用 exact resume service。
- `backend/crates/golish-db/tests/runtime_memory_worker_transactions.rs`：same-worker continuation 在冻结 worker budget 下仍可 claim 的持久化回归。
- `docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/backend/golish-agent-app/ai.md`、
  `docs/modules/backend/golish/stage_run.md`、`docs/modules/INDEX.md`：同步模块合同。
- `agent-progress.md`、`feature_list.json`：记录 fresh RED/GREEN、CLI/DB/transcript 证据和未运行的 broad gate。

## Task 1：RED—锁定 Controller 必须 drain safe-reconciled child

**文件：** 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`

**步骤：**

1. 在 tests 模块新增 `company_controller_waiting_action_drains_live_reconciled_child`，构造
   `recovery_required_workers=0, live_workers=1, retry_pending_work_items=0` 的 barrier，断言结果是
   `DrainChildren`。
2. 新增 `company_controller_waiting_action_keeps_operator_recovery_terminal`，构造
   `recovery_required_workers=1`，断言 typed operator recovery 优先于 drain。
3. 运行 focused test，确认旧实现因缺少三态/仍直接返回 no-runnable 而 RED。

**验证：**

```bash
just space-guard
cd backend && cargo test -p golish-agent-runtime company_controller_waiting_action --lib -- --nocapture
```

预期 RED：新断言失败或缺少 `DrainChildren` 行为，不能因测试 setup/拼写错误失败。

**提交：** 不提交；用户未要求 commit，保留共享 dirty tree。

## Task 2：GREEN—leader-none 先 drain child，再重领 Controller

**文件：** 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`

**步骤：**

1. 定义 closed action：

```rust
enum CompanyControllerWaitingAction {
    DrainChildren,
    OperatorRecoveryRequired { workers: i64 },
    NoRunnableChild,
}
```

2. helper 必须让 operator recovery 优先；否则 barrier 有 live/retry child 时返回 `DrainChildren`。
3. `execute_company_controller_unit` 的 leader-none 分支先执行 action。Drain 时调用现有
   `drain_company_controller_children`；`completed > 0` 后 `continue`，0 时 reload exact barrier 再判定，禁止 busy loop。
4. 保留 `turn_budget`、provider semaphore、WorkItem/Worker fence 和 typed recovery error，不新建执行旁路。

**验证：**

```bash
just space-guard
cd backend && cargo test -p golish-agent-runtime company_controller_waiting_action --lib -- --nocapture
cd backend && cargo nextest run -p golish-agent-runtime -E 'test(company_controller) | test(interrupted_crawler_recovery)' --status-level fail
```

预期 GREEN：focused tests 全过，既有 operator recovery/Controller tests 无回归。

**提交：** 不提交。

## Task 3：RED/GREEN—同一 queued Worker 不被 distinct-worker budget 拦截

**文件：** 修改 `backend/crates/golish-db/tests/runtime_memory_worker_transactions.rs`、
`backend/crates/golish-db/src/repo/runtime_memory_tx.rs`

**步骤：**

1. 新增 `queued_same_worker_resume_bypasses_distinct_worker_lifetime_cap`：把 Team 的
   `max_workers_total` 收紧到当前 WorkerRun 数，重排一个 existing Worker 为 queued，断言再次 claim 返回同 Worker/chain、
   epoch+1，而不是 `stage_team_worker_lifetime_budget_exhausted`。
2. 先运行测试确认 RED。
3. 在 claim transaction 内先识别 exact WorkItem 的 resumable queued Worker；distinct-worker total cap 只保护插入新
   WorkerRun 的分支。active cap 不得把该 queued Worker 自己算成阻止自己的额外并发槽。
4. 重跑 focused DB recovery tests确认 GREEN。

**验证：**

```bash
just space-guard
cd backend && cargo nextest run -p golish-db -E 'test(queued_same_worker_resume_bypasses_distinct_worker_lifetime_cap) | test(company_controller_continue_reclaims_interrupted_eas_child_on_same_chain)' --status-level fail
```

**提交：** 不提交。

## Task 4：RED—CLI 必须接受 exact GUI operation 并追加 durable Turn

**文件：** 修改 `backend/crates/golish/src/stage_run/mod.rs`

**步骤：**

1. 把现有 candidate fixture 的 chat key 改成 `pentest-chat-*`，新增
   `resume_candidate_accepts_exact_gui_task_session`，断言 exact operation/session 路径不因前缀拒绝。
2. 新增 `stage_run_resume_uses_shared_operation_turn_claim`，锁定 CLI orchestration 调用共享
   `select_exact_resume_runtime_source` / `claim_exact_resume_runtime_source`，而不是旧
   `claim_exact_resume_task`。
3. 运行 focused CLI tests，确认当前 stage-run-only 与旧 claim 路径 RED。

**验证：**

```bash
just space-guard
cd backend && cargo test -p golish stage_run::tests::resume_candidate_accepts_exact_gui_task_session --lib -- --nocapture
cd backend && cargo test -p golish stage_run::tests::stage_run_resume_uses_shared_operation_turn_claim --lib -- --nocapture
```

**提交：** 不提交。

## Task 5：GREEN—CLI/GUI 复用 exact source + open-Turn CAS

**文件：** 修改
`backend/crates/golish-agent-app/src/ai/commands/core/operation_resume.rs`、
`backend/crates/golish-agent-app/src/ai/commands/core/mod.rs`、
`backend/crates/golish/src/stage_run/mod.rs`

**步骤：**

1. 以公开 Rust service 类型导出 `SelectedExactResume` 及 select/claim；不暴露 Tauri command 或 caller-authored
   source/open-turn 值。
2. CLI selector 接受 exact `stage-run-*` 与 `pentest-chat-*` session；UUID 路径仍通过 session/task/operation/project/
   workspace/profile/stage/org/provider/model/chain/tool fence 校验。
3. 正常 running/waiting path不再要求先 repair成 waiting。advisory lock保持持有；bridge/repo/project scope全部 ready 后，
   调共享 claim并把 selected source绑定 orchestrator/bridge，再调用旧 operation 的 `resume()`。
4. failed startup-reaper repair仍是显式异常路径；修复后必须存在 exact open Turn，否则 shared select fail closed。
5. 不创建 task/operation/session，不调用 `run_stage`，不改 migration/schema。

**验证：**

```bash
just space-guard
cd backend && cargo test -p golish stage_run::tests::resume_candidate --lib -- --nocapture
cd backend && cargo nextest run -p golish-agent-app -E 'test(operation_resume)' --status-level fail
cd backend && cargo nextest run -p golish-db -E 'test(exact_resume_source_claim) | test(exact_resume_claims_running_v2_operation)' --status-level fail
```

预期：GUI/CLI source/Turn claim tests 全绿，running operation 无 reaper delay，竞争 caller只有一个 successor Turn。

**提交：** 不提交。

## Task 6：真实 CLI 接管当前 operation 并验证 EAS 闭环

**文件：** 不预设生产代码修改；根据新鲜失败证据只做同 scope TDD 修复。

**步骤：**

1. build 前运行 `just space-guard`，构建当前 `golish` binary。
2. 在没有 GUI/旧 Golish 进程的前提下，用 workspace Test1、operation UUID、exact session/task/org/stage 运行
   `--stage-run-resume`；provider/model沿用 persisted DeepSeek 配置。
3. 运行 `scripts/run_tree.py --workspace ... --full --db <session>`，并查询 operation Turns、Worker epoch/chain、旧 tool
   terminal status、Gate outcome。
4. 若失败，先保存 exact run.log/transcript/DB blocker，写 RED，再做最小修复并从同一 durable operation继续；不得 fresh restart。

**验证：** CLI exit 0 或明确的 deterministic Gate terminal；必须不存在 reconcile 后
`COMPANY_CONTROLLER_FAILED`，并有 same-operation/same-chain/new-Turn DB 证据。

**提交：** 不提交。

## Task 7：同步模块卡、progress 与 scoped 门禁

**文件：** 修改上述模块卡、`docs/modules/INDEX.md`、`agent-progress.md`、`feature_list.json`

**步骤：**

1. 记录 leader-none child drain、CLI/GUI shared Turn claim 与真实 acceptance IDs。
2. 运行 rustfmt、相关 package Clippy、focused suites、JSON 与 diff hygiene。
3. 按用户要求不运行 `init.sh`；未跑 `just precommit` 时父 feature 保持 `in_progress`。

**验证：**

```bash
just space-guard
cd backend && cargo fmt -p golish-agent-runtime -p golish-agent-app -p golish-db -p golish -- --check
cd backend && cargo clippy -p golish-agent-runtime -p golish-agent-app -p golish-db -p golish --all-targets -- -D warnings
jq empty feature_list.json
git diff --check
```

**提交：** 不提交、stage 或 push，除非用户另行要求。

## Task 6 实跑检查点（2026-07-16）

- [x] exact GUI session/task/operation 已由 CLI shared Turn claim 接管；Turn 5 interrupted、Turn 6 running。
- [x] reconciled Prober 保持 WorkItem/WorkerRun/chain，epoch 2 完成；原 Controller 保持 WorkerRun/chain，epoch 3
  完成一次 repair coordination。
- [x] 第一轮四个 service gap 已写 evidence；第二轮 Gate 收敛到 70/72、三个 exact Web origins。
- [x] 历史 WorkItem epoch replay mismatch 已按 RED→GREEN 修复并覆盖 static/dynamic/gate-repair authority。
- [x] 普通 Controller Gate BLOCK 已成为 typed request halt，避免外层 Agent 在同一 request 旁路 submit。
- [x] 用户已明确同意 forward migration；successor Turn 7 通过 exact authority 恢复同一 plan/unit/leader/Controller
  WorkerRun/message chain，并以 EAS Gate PASS、CLI exit 0 收口。

## Task 8：获批后实现 successor-Turn Controller reopen migration

**高风险门禁：** 用户已在当前会话明确回复“同意迁移”，本任务据此执行；未扩展到其它 schema。

**预期文件：** 新 migration、`runtime_memory_tx.rs`、对应 DB/runtime integration、module cards。

**步骤：**

1. RED：从 closed epoch、superseded leader、同 Controller 已有 gap 的真实形态开始，successor Turn resume 当前应分别命中
   `STAGE_TEAM_REQUEST_EPOCH_CANNOT_REOPEN`、invalid WorkItem transition 或 duplicate gap source。
2. 新增 exact operation-Turn Controller resume authority；所有 identity、from/to epoch、checkpoint hash 和 old/new Turn
   都由 server transaction 冻结。
3. forward-replace plan/work-item triggers：只有 exact unconsumed authority 才允许 plan reopen 和 stable leader reactivation；
   其它 caller 保持原拒绝。
4. 把 gap source WorkerRun 的单列 UNIQUE 改为保存每轮 immutable gap 的组合约束/索引，不更新或删除旧 gap。
5. compound transaction 复用同 plan/unit/item/WorkerRun/message chain并消费 authority；response-loss replay 返回相同结果。
6. 重建 CLI，先停止 repo-scoped GUI backend，继续当前 operation 新 Turn，处理三个 exact origin直到 deterministic Gate
   PASS，或记录一个新的、非同类 durable blocker后继续 TDD。
7. 跑 focused DB/runtime/CLI suites、`run_tree.py --full --db`、scoped Clippy/rustfmt/JSON/diff；按用户要求仍不跑
   `init.sh`。

**完成证据（2026-07-16）：**

- [x] RED legacy no-gap continuation 精确返回 `stage_team_controller_turn_resume_legacy_gap_missing`；GREEN focused DB
  continuation 覆盖 exact authority、identity 保持、attempt 推进及 response-loss replay。
- [x] migration 在真实 embedded PostgreSQL 成功应用，目标 operation 由 Turn 6 进入 successor Turn 7；同一 stage execution、
  plan、unit、leader、Controller WorkerRun 和 message chain 均保持不变。
- [x] CLI 仅运行 EAS 剩余 worklist，authoritative Gate=`PASS`，进程 exit 0，并输出原 transcript 路径与 DB smoke summary。
- [x] `cargo nextest run -p golish-db --no-fail-fast --status-level fail`：576/576 passed。
- [x] 按用户要求未运行 `init.sh`。
