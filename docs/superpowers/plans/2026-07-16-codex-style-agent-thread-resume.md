# Codex 式 Agent Thread / Turn 恢复实现计划

> Active-tool 一律人工恢复的步骤已由
> `2026-07-16-stage-team-interrupted-tool-reconciliation.md` 的 closed bounded-crawler 计划部分取代；其余任务保持有效。

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 让顶层 Operation、Company Controller 和每个 child 在崩溃/等待后沿原 message chain追加新 Turn，而不是创建空白 replacement Agent。

**架构：** `tasks + operation_state`保持稳定 Operation Thread，新增`operation_turns`记录每次顶层执行；Stage Team复用现有 WorkItem/WorkerRun/message chain，并以`attempt_epoch`推进 Turn。exact resume、lease recovery和chain restore全部使用DB CAS，outcome-unknown工具继续fail closed。

**技术栈：** Rust 2021、sqlx/PostgreSQL、rig message chains、Tauri/React/TypeScript、cargo nextest、Vitest。

## 文件结构

- `docs/design/2026-07-16-codex-style-agent-thread-resume.md`：用户合同、Thread/Turn映射、安全边界。
- `backend/crates/golish-db/migrations/20260716000001_operation_turn_resume.sql`：`operation_turns`表、约束和历史active operation回填。
- `backend/crates/golish-db/src/repo/operation_turns.rs`：turn查询、创建、terminal transition。
- `backend/crates/golish-db/src/repo/tasks.rs`：running/waiting共用的exact source选择与原子resume claim。
- `backend/crates/golish-db/src/repo/runtime_memory_tx.rs`：fresh turn创建、Team child同Worker/chain reclaim。
- `backend/crates/golish-db/src/repo/stage_teams.rs`：startup reaper复用同一Worker并按attempt epoch计费。
- `backend/crates/golish-agent-app/src/ai/commands/core/operation_resume.rs`：一次调用完成source pin与turn claim。
- `backend/crates/golish-sub-agents/src/executor/inner.rs`：只在测试需要时强化“恢复追加、不重放initial objective”合同。
- `frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`：用现有attempt epoch/read model显示同卡恢复状态（如果read model已有字段，不新增IPC）。
- 对应模块卡、`docs/modules/INDEX.md`、`agent-progress.md`、`feature_list.json`：同步事实与证据。

## Task 1：RED—锁定顶层 running operation可立即继续

**文件：**

- 修改 `backend/crates/golish-db/src/repo/tasks.rs`
- 修改 `backend/crates/golish-db/src/repo/runtime_memory_tx.rs`

**步骤：**

1. 在SQL shape测试中把期望从仅`tasks.status='waiting'`改为：

   ```rust
   assert!(sql.contains("tasks.status IN ('running','waiting')"));
   assert!(sql.contains("lease_expires_at>NOW()"));
   ```

2. 新增集成测试`running_operation_without_live_worker_lease_claims_next_turn_immediately`：创建V2 operation和
   complete relational state，保持task=`running`、worker lease过期，调用新的claim API并断言source=`V2`。
3. 运行：

   ```bash
   just space-guard
   cd backend && cargo nextest run -p golish-db -E 'test(running_operation_without_live_worker_lease_claims_next_turn_immediately)' --status-level fail
   ```

   预期RED：旧selector返回`None`或旧claim因task不是`waiting`失败。

**验证：** RED失败原因必须是状态合同缺失，不是migration/fixture拼写错误。

**提交：** `test(db): reproduce immediate operation resume refusal`

## Task 2：GREEN—增加operation turns并原子claim下一Turn

**文件：**

- 创建 `backend/crates/golish-db/migrations/20260716000001_operation_turn_resume.sql`
- 创建 `backend/crates/golish-db/src/repo/operation_turns.rs`
- 修改 `backend/crates/golish-db/src/repo/mod.rs`
- 修改 `backend/crates/golish-db/src/repo/tasks.rs`
- 修改 `backend/crates/golish-db/src/repo/runtime_memory_tx.rs`
- 修改 `backend/crates/golish-agent-app/src/ai/commands/core/operation_resume.rs`
- 修改 `backend/crates/golish-agent-app/src/ai/commands/core/chat.rs`

**步骤：**

1. migration创建设计文档中的`operation_turns`，并为已有非terminal operation回填ordinal 1；terminal task回填
   `completed`或`failed`，running/waiting保持open。
2. fresh operation transaction在task/operation_state创建后写入：

   ```rust
   operation_turns::insert_initial_with_executor(
       &mut *tx,
       operation_id,
       input,
   ).await?;
   ```

3. preflight在真实解码完整source后返回`source + exact open_turn_id`，claim SQL把两者都作为CAS witness：

   ```rust
   pub async fn claim_exact_resumable_runtime_source(
       pool: &PgPool,
       task_id: Uuid,
       session_id: Uuid,
       expected_source: RuntimeMemoryRecordSource,
       expected_open_turn_id: Uuid,
       next_turn_id: Uuid,
       trigger_input: &str,
   ) -> Result<bool>;
   ```

   transaction内锁task、重算完整source、拒绝live Worker lease、关闭旧open turn、插入下一ordinal并把task CAS为running。
4. app在preflight与claim之间不信任旧快照：claim重新计算source并只关闭exact open turn；任一witness漂移都返回false。
   成功后把同一pinned source绑定到orchestrator和bridge。
5. 重跑Task 1测试，预期GREEN；再加并发测试，两个claim future只有一个成功创建ordinal 2。

**验证：**

```bash
cd backend && cargo nextest run -p golish-db -E 'test(operation_turn) | test(exact_resumable_runtime_source)' --status-level fail
cd backend && cargo nextest run -p golish-agent-app -E 'test(operation_resume)' --status-level fail
```

**提交：** `feat(runtime): claim durable operation turns on resume`

## Task 3：RED—锁定expired child必须保留Worker和chain

**文件：**

- 修改 `backend/crates/golish-db/src/repo/runtime_memory_tx.rs`
- 修改 `backend/crates/golish-db/src/repo/stage_teams.rs`

**步骤：**

1. 新增`expired_clean_child_resumes_same_worker_and_message_chain`，保存首次claim的
   `work_item.id / worker.id / message_chain_id / attempt_epoch`，使lease过期后再次claim。
2. 断言：

   ```rust
   assert_eq!(resumed.work_item.id, first.work_item.id);
   assert_eq!(resumed.worker.id, first.worker.id);
   assert_eq!(resumed.message_chain_id, first.message_chain_id);
   assert_eq!(resumed.worker.attempt_epoch, first.worker.attempt_epoch + 1);
   ```

3. 运行聚焦测试，预期RED：旧实现返回新的WorkerRun和chain。

**验证：** 测试必须展示identity变化，而不是只检查status。

**提交：** `test(stage-team): reproduce fresh child chain after lease expiry`

## Task 4：GREEN—统一request-time/startup同链reclaim

**文件：**

- 修改 `backend/crates/golish-db/src/repo/runtime_memory_tx.rs`
- 修改 `backend/crates/golish-db/src/repo/stage_teams.rs`
- 修改 `backend/crates/golish-db/src/repo/stage_worker_runs.rs`

**步骤：**

1. 提取attempt计费SQL为所有Team recovery共用：

   ```sql
   SELECT COALESCE(SUM(GREATEST(attempt_epoch, 1)), 0)
     FROM stage_worker_runs
    WHERE work_item_id=$1
   ```

2. 无active tool时把原Worker改为`queued`，清lease/heartbeat/terminal marker但保留
   `message_chain_id/checkpoint/checkpoint_version`；WorkItem回到`queued`。
3. `claim_stage_team_item`选中WorkItem后优先锁定同一WorkItem的queued Worker；存在时调用`claim_cas`递增
   attempt epoch并直接返回旧chain，只有从未创建Worker时才插入新Worker/chain。
4. startup reaper调用相同transition；attempt fuel耗尽时才把Worker/WorkItem终态化并写blocked output。
5. active tool路径保持`recovery_required`，增加断言证明tool row和chain均未重放。

**验证：**

```bash
cd backend && cargo nextest run -p golish-db -E 'test(expired_clean_child_resumes_same_worker_and_message_chain) | test(stage_team_worker_recovery)' --status-level fail
```

**提交：** `fix(stage-team): resume expired children on their exact chain`

## Task 5：验证executor和UI仍指向同一Thread

**文件：**

- 修改 `backend/crates/golish-sub-agents/src/executor/chain_persist.rs`
- 修改 `frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts`
- 仅当测试要求时修改 `frontend/components/SubAgentDetailView/SubAgentDetailView.tsx`

**步骤：**

1. 增executor测试：bound worker的第二attempt加载旧messages，追加一个resume directive，不再次插入initial objective。
2. 增前端测试：相同worker identity、attempt epoch增加仍只有一张child卡；旧Thought与新Turn按一条时间线展示。
3. 若现有UI已经满足，保留生产组件不变并记录“测试证明无需改IPC/UI”。

**验证：**

```bash
cd backend && cargo nextest run -p golish-sub-agents -E 'test(bound_worker_resume)' --status-level fail
pnpm exec vitest run frontend/components/SubAgentDetailView/stripAgentXmlTags.test.ts
```

**提交：** `test(ui): keep resumed subagent on one timeline`

## Task 6：同步文档、状态与聚焦验证

**文件：**

- 修改 `docs/modules/backend/golish-db/repo.md`
- 修改 `docs/modules/backend/golish-agent-kit/task_orchestrator.md`
- 修改 `docs/modules/backend/golish-agent-app/ai.md`
- 修改 `docs/modules/backend/golish-agent-runtime/agentic_loop.md`
- 修改 `docs/modules/backend/golish-sub-agents/executor.md`
- 修改 `docs/modules/frontend/components.md`（仅UI行为变化时）
- 修改 `docs/modules/INDEX.md`
- 修改 `agent-progress.md`
- 修改 `feature_list.json`

**步骤：**

1. 记录Thread/Turn ownership、running/waiting exact claim、same-worker chain recovery和unknown-tool边界。
2. 更新active feature notes；缺少live app rerun或broad suite时保持`in_progress`。
3. 按用户要求不跑`init.sh`和broad precommit，只跑：

   ```bash
   just space-guard
   cd backend && cargo fmt -p golish-db -p golish-agent-app -p golish-agent-runtime -p golish-sub-agents -- --check
   cd backend && cargo check -p golish-db -p golish-agent-app -p golish-agent-runtime -p golish-sub-agents --tests
   jq empty feature_list.json
   git diff --check
   ```

**验证：** progress记录每条命令、exit code、测试数量和未运行的broad gate。

**提交：** `docs(runtime): record Codex-style resume evidence`
