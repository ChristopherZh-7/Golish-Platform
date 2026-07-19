# Stage Team repair epoch 派工与失败状态收敛实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:executing-plans 逐任务实现此计划，并使用 test-driven-development 完成每个 RED→GREEN 循环。

**目标：** 让 Gate repair 新 epoch 中的稳定 Company Controller 可以继续派发 durable child，并让所有未落地的派工失败在 UI 中收敛为 error。

**架构：** additive migration 把 parent WorkItem owner identity 与 Request generation 解耦，同时由 DB trigger 和 Rust transaction 双重验证 exact repair/resume authority；Request 与 accepted child 仍绑定当前开放 epoch。前端只把终态 tool failure 投影成 dispatch error，不伪造 durable rejection。

**技术栈：** PostgreSQL、SQLx、Rust 2021、React 19、TypeScript、Vitest、cargo-nextest。

---

## 文件结构

- 新建 `backend/crates/golish-db/migrations/20260717000002_stage_team_repair_epoch_dispatch.sql`：替换 parent FK 与 worker-request trigger，保留当前 epoch/owner/repair authority。
- 修改 `backend/crates/golish-db/src/repo/runtime_memory_tx.rs`：在插入前显式验证跨 epoch stable Controller authority并返回 typed conflict。
- 修改 `backend/crates/golish-db/tests/runtime_memory_worker_transactions.rs`：真实 Gate repair epoch dispatch 与 negative regressions。
- 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs`：保持 persistence error typed payload，并覆盖 repair dispatch router 行为。
- 修改 `frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`：终态 dispatch failure 的 assignment 状态。
- 修改 `frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts`：截图同形态 UI regression。
- 修改 `docs/modules/backend/golish-db.md`、`docs/modules/backend/golish-db/repo.md`、`docs/modules/backend/golish-agent-runtime.md`、`docs/modules/backend/golish-agent-runtime/agentic_loop.md`、`docs/modules/frontend/components.md`、`docs/modules/INDEX.md`：同步合同。
- 修改 `feature_list.json`、`agent-progress.md`：登记状态和验证证据。

### Task 1：建立真实 repair epoch RED 与 UI RED

**文件：**

- 修改：`backend/crates/golish-db/tests/runtime_memory_worker_transactions.rs`
- 修改：`frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts`

**步骤：**

1. 复用 Company Controller fixture：claim `leader:primary`，关闭 epoch 0，构造 deterministic Gate gap，调用
   `reopen_stage_team_leader_after_gate_block`，再 claim 同一个 Controller。
2. 用当前 plan epoch 1、旧 leader WorkItem id 创建 `RequestStageWorkerRow`，断言修复前返回
   `RuntimeMemoryStoreError::Sqlx` 且数据库 constraint 为 parent WorkItem epoch FK。
3. 增加 negative fixture：没有 current `building|sealed` repair generation 或 applied successor authority 的跨 epoch parent 必须返回 typed
   `stage_team_controller_parent_epoch_not_authorized`。
4. 增加 frontend fixture：`stage_team_dispatch_workers` status=`error`、result code=
   `STAGE_TEAM_DISPATCH_PERSIST_FAILED`、无 `requests[]`，断言 args-derived assignment 为 error、queued 为零。
5. 每次 Cargo 前运行 `just space-guard`，再运行 focused nextest/Vitest，确认新增目标断言 RED。

**验证：**

```bash
just space-guard
cd backend && cargo nextest run -p golish-db -E 'test(stage_team_repair_epoch_dispatch)' --status-level fail
pnpm exec vitest run frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts
```

预期：DB 因旧 FK 失败；frontend 因旧实现显示 queued 而失败。

### Task 2：实现 additive DB contract 与 transaction guard

**文件：**

- 新建：`backend/crates/golish-db/migrations/20260717000002_stage_team_repair_epoch_dispatch.sql`
- 修改：`backend/crates/golish-db/src/repo/runtime_memory_tx.rs`
- 修改：`backend/crates/golish-db/tests/runtime_memory_worker_transactions.rs`

**步骤：**

1. migration 通过 catalog 查出并删除 `stage_worker_requests` 唯一包含
   `parent_work_item_id + dispatch_epoch` 的 FK，再创建不含 epoch 的 parent owner FK。
2. `CREATE OR REPLACE FUNCTION enforce_stage_worker_request_contract()`：Request 必须等于 current open plan epoch；
   parent epoch 不同只允许 exact Company Controller leader，且 current epoch 必须有 `building|sealed` repair generation 或
   applied successor-Turn authority绑定同一 leader/WorkerRun。
3. Rust 新增 `validate_stage_team_request_parent_epoch_authority`，普通 parent 同 epoch；Controller 跨 epoch时查
   durable repair/resume authority，失败返回 `stage_team_controller_parent_epoch_not_authorized`。
4. 保留 accepted child 的 current-epoch FK、dynamic request scope validation、request hash/dedupe、lease/checkpoint
   CAS 不变。
5. RED test 改为断言 GREEN：Request 与 child epoch=1、parent epoch=0；negative tests 保持 typed rejection。

**验证：**

```bash
just space-guard
cd backend && cargo nextest run -p golish-db -E 'test(stage_team_repair_epoch_dispatch)' --status-level fail
```

预期：合法 repair dispatch 通过，全部 negative cases 通过，测试数据库 migration 正常安装。

### Task 3：修复 runtime/UI 失败收敛

**文件：**

- 修改：`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs`
- 修改：`frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`
- 修改：`frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts`

**步骤：**

1. runtime 对 repo persistence failure 保留稳定 `STAGE_TEAM_DISPATCH_PERSIST_FAILED`、error 与
   `accepted_count=0/request_count`，不返回 raw-only result，也不 park Controller。
2. frontend 在没有逐项 decision/accepted child 时，根据 tool terminal error 投影 assignment error；若已有
   accepted request 或 nested Agent，仍优先 durable/运行真值。
3. 在 assignment 卡内展示稳定 tool code/error，避免用户只看到排队状态。
4. 运行 runtime focused test 与 frontend test，确认 GREEN。

**验证：**

```bash
just space-guard
cd backend && cargo nextest run -p golish-agent-runtime -E 'test(stage_team_dispatch)' --status-level fail
pnpm exec vitest run frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts
```

预期：runtime 不进入 waiting barrier；UI error 卡数量等于未落地 workers 数量，queued 为零。

### Task 4：文档同步与聚焦验证

**文件：**

- 修改：`docs/modules/backend/golish-db.md`
- 修改：`docs/modules/backend/golish-db/repo.md`
- 修改：`docs/modules/backend/golish-agent-runtime.md`
- 修改：`docs/modules/backend/golish-agent-runtime/agentic_loop.md`
- 修改：`docs/modules/frontend/components.md`
- 修改：`docs/modules/INDEX.md`
- 修改：`feature_list.json`
- 修改：`agent-progress.md`

**步骤：**

1. 记录 parent identity 与 Request generation 分离、exact repair authority、typed persistence failure 和 UI
   error 收敛。
2. 运行 affected DB/runtime/frontend tests、migration tests、cargo check/Clippy、rustfmt、TypeScript/Biome、
   JSON/diff checks。
3. 按用户要求不运行 `init.sh`；不调用 provider、scanner、真实目标或外部 API。
4. 共享 dirty tree 不自动 stage/commit/push。

**验证：**

```bash
just space-guard
cd backend && cargo nextest run -p golish-db -E 'test(stage_team_repair_epoch_dispatch) | test(runtime_memory_migrations)' --status-level fail
cd backend && cargo nextest run -p golish-agent-runtime -E 'test(stage_team_dispatch)' --status-level fail
cd backend && cargo check -p golish-db -p golish-agent-runtime
cd backend && cargo clippy -p golish-db -p golish-agent-runtime --all-targets -- -D warnings
cargo fmt --manifest-path backend/Cargo.toml --all -- --check
pnpm exec vitest run frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts
pnpm typecheck
pnpm exec biome check frontend/components/SubAgentDetailView/SubAgentDetailView.tsx frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts
jq empty feature_list.json
git diff --check
```

预期：全部 exit 0、无 warning。

### Task 5：完整门禁与证据记录

**文件：**

- 修改：`feature_list.json`
- 修改：`agent-progress.md`

**步骤：**

1. 运行 `just precommit`，记录退出码和关键测试摘要。
2. 对照 `clean-state-checklist.md`，记录共享 dirty tree 与未提交文件，不清理其它功能改动。
3. 只有 verification、precommit、文档和证据全部满足时把 feature 设为 `passing`；否则保留
   `in_progress`/`blocked` 并写明缺口。

**验证：**

```bash
just precommit
```

预期：打印 `All checks passed!` 且 exit 0。
