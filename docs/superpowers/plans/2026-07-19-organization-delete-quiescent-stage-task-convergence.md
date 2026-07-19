# Organization Delete Quiescent Stage Task Convergence 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 让明确确认的组织删除原子终结没有 durable executor authority 的 `waiting` stage-fork Task，同时继续拒绝真正运行中或 outcome-unknown 的任务。
**架构：** 组织删除事务在既有 Organization/Target 锁之后，以 operation→Task→Worker 的稳定顺序锁定 stage-fork authority。只对零有效 lease、零 active-tool pointer 的 `waiting` Task 关闭 stale tool、Task 和 Turn，再继续创建既有 two-phase deletion job；`created|running` 或 recovery-required 保持 typed blocker。
**技术栈：** Rust 2021、sqlx/PostgreSQL、Tauri 2、React 19、TypeScript、Vitest、Biome。

## 文件结构

- `backend/crates/golish-db/src/repo/organization_deletion_jobs.rs`：实现 quiescent stage-fork 分类、稳定加锁和同事务终结。
- `backend/crates/golish-db/tests/operation_stage_forks.rs`：fresh embedded-Postgres RED→GREEN 行为与 race/fence 回归。
- `frontend/lib/api/error-codes.ts`、`frontend/lib/api/error-codes.test.ts`：剩余 blocker 的精确用户提示。
- `frontend/lib/i18n/en.json`、`frontend/lib/i18n/zh-CN.json`：删除确认披露 paused Task 自动终结。
- `frontend/components/TargetPanel/TargetGroupedView.delete.test.tsx`：确认弹窗和 typed blocker 交互回归。
- `docs/modules/backend/golish-db.md`、`docs/modules/backend/golish-db/repo.md`、`docs/modules/frontend/components.md`、`docs/modules/frontend/lib.md`、`docs/modules/INDEX.md`：同步 system-of-record。
- `feature_list.json`、`agent-progress.md`：唯一 active feature、验证证据与交接状态。

## 任务 1：建立失败回归

**文件：** 修改 `backend/crates/golish-db/tests/operation_stage_forks.rs`、`frontend/components/TargetPanel/TargetGroupedView.delete.test.tsx`、`frontend/lib/api/error-codes.test.ts`。

**步骤 1：** 在现有 stage-fork 集成场景中先保持 `created` Task 删除拒绝；随后把同一 Task 设为 `waiting`，插入一个 `running` parent `stage_run` tool，重新请求删除并断言：

```rust
assert_eq!(task_status, "failed");
assert_eq!(tool_status, "failed");
assert_eq!(open_turn_count, 0);
assert_eq!(accepted_deletion.state, "waiting_for_invalidation_delivery");
```

**步骤 2：** 更新前端断言，要求确认文案包含 paused stage Task 自动停止说明，剩余 blocker 文案包含 active executor 或 unresolved tool outcome。

**验证：**

```bash
just space-guard
cd backend && cargo nextest run -p golish-db --test operation_stage_forks -E 'test(shared_db_candidate_fork_materializes_scoping_prefix_targets_and_wave_entry)' --status-level fail
pnpm exec vitest run frontend/components/TargetPanel/TargetGroupedView.delete.test.tsx frontend/lib/api/error-codes.test.ts
```

预期：后端测试因旧实现仍拒绝 quiescent `waiting` fork 而失败；前端测试因旧文案缺少新语义而失败。此时不修改生产实现。

**提交：** 本仓库当前是用户共享 dirty tree，且用户未要求提交；不 stage/commit，只在 `agent-progress.md` 记录 RED 命令、退出码和关键失败。

## 任务 2：实现最小后端收敛

**文件：** 修改 `backend/crates/golish-db/src/repo/organization_deletion_jobs.rs`。

**步骤 1：** 新增稳定结果常量和 locked fork row：

```rust
const ORGANIZATION_DELETION_STOPPED_TASK_RESULT: &str =
    "Stopped: organization deletion closed a quiescent stage task.";

#[derive(Clone, Debug, sqlx::FromRow)]
struct LockedStageForkTaskRow {
    operation_id: Uuid,
    stage: String,
    status: String,
}
```

**步骤 2：** 在 Target 锁之后查询 exact fork operation ids，按 UUID 锁 `operation_state`、Task 和 Worker rows。分类规则固定为：

```rust
let quiescent = task.status == "waiting"
    && workers.iter().all(|worker| {
        worker.active_tool_call_id.is_none()
            && worker.lease_expires_at.is_none_or(|expires| expires <= Utc::now())
    });
```

实现时在 SQL 内使用同一 PostgreSQL `NOW()` 快照计算 `has_live_lease`，避免应用时钟参与权威判断；任何 `created|running`、live lease 或 active pointer 返回现有 `OrganizationDeletionActiveStageFork`。

**步骤 3：** 对全部 quiescent operation ids 先执行：

```sql
UPDATE tool_calls
   SET status='failed',
       result=COALESCE(result, $2),
       updated_at=NOW()
 WHERE task_id=ANY($1)
   AND operation_id=ANY($1)
   AND status IN ('received','running');
```

再以 `status='waiting'` CAS 把对应 Task 设为 `failed` 和稳定 result；affected row count 必须等于计划终结数，否则事务失败。既有 Task trigger 自动把 open Turn 设为 failed。

**步骤 4：** 将 stopped operation ids/count 写入 deletion job 的 `deleting_db_committed` history detail，不新增表/列或 IPC 类型。

**验证：** 重跑任务 1 的后端命令，预期 1/1 passed；再运行同文件相邻测试，确认 active-fork 和 reverse deletion fence 仍通过。

**提交：** 不 stage/commit；记录 GREEN run id 和关键断言。

## 任务 3：更新用户提示

**文件：** 修改 `frontend/lib/api/error-codes.ts`、`frontend/lib/i18n/en.json`、`frontend/lib/i18n/zh-CN.json`。

**步骤 1：** 将剩余 typed blocker 文案改为：

```ts
ORGANIZATION_DELETE_ACTIVE_STAGE_FORK:
  "A stage task still has an active executor or unresolved tool outcome. Stop or recover it before deleting.",
```

**步骤 2：** 在中英文 `organizations.deleteConfirm` 的不可恢复提示后追加：没有 active executor 的 paused stage Tasks 会被停止；active/recovery-required Task 仍会阻止删除。

**验证：** 重跑任务 1 的 Vitest，预期全部通过。

**提交：** 不 stage/commit；记录 focused frontend evidence。

## 任务 4：同步文档和完成定向验证

**文件：** 修改上列五张模块卡/索引、`feature_list.json`、`agent-progress.md`。

**步骤 1：** 记录 DB authority、锁顺序、quiescent 条件、Task/tool/Turn closure 和 UI 文案；不改模块职责层级，只更新卡片当前行为和 INDEX synchronization note。

**步骤 2：** 运行：

```bash
just space-guard
cd backend && cargo nextest run -p golish-db --test operation_stage_forks -E 'test(shared_db_candidate_fork_materializes_scoping_prefix_targets_and_wave_entry)' --status-level fail
cd backend && cargo clippy -p golish-db --test operation_stage_forks -- -D warnings
cd backend && cargo fmt -p golish-db -- --check
pnpm exec vitest run frontend/components/TargetPanel/TargetGroupedView.delete.test.tsx frontend/lib/api/error-codes.test.ts
pnpm exec biome check frontend/components/TargetPanel/TargetGroupedView.delete.test.tsx frontend/lib/api/error-codes.ts frontend/lib/api/error-codes.test.ts frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json
pnpm exec tsc --noEmit --pretty false
jq empty feature_list.json frontend/lib/i18n/en.json frontend/lib/i18n/zh-CN.json
git diff --check
```

预期：所有命令 exit 0；Clippy 零 warning。未获授权，不运行 `init.sh`、`just precommit` 或全 workspace 测试。

**步骤 3：** 逐项对照设计：active/recovery blocker、quiescent closure、atomic deletion admission、source preservation、UI disclosure 全部有 fresh evidence 后，才把新 feature 设为 `passing`；否则保持 `in_progress` 并记录剩余风险。

**提交：** 用户未要求提交，且共享树有大量既有未提交改动；不 stage/commit/push。
