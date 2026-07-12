# Golish 运行期记忆底座实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 冻结每个 operation 的公司范围，并把 stage/org/worker checkpoint 与 PASS handoff 从共享 `operation_state.state_blob` 迁到可约束、可恢复、可审计的表。

**架构：** `operation_state` 保留 coarse cursor；`stage_runs.id` 是 durable `stage_execution_id`；`stage_run_units` 表示每次 execution 的 per-org final Gate 单元；内部 wave 由既有 wave 表和 WorkerRun work item 表达；`stage_worker_runs` 使用 DB lease + CAS 保存独立 chain/checkpoint；`stage_handoffs` 只在 org-stage final seal 后发布。先 additive + 原子 dual-write，再切 V2 权威读，不在本计划删除 legacy 字段。

**技术栈：** Rust 2021、sqlx/PostgreSQL、golish-db、golish-agent-kit、golish-agent-app、golish-agent-runtime、golish-sub-agents、cargo-nextest。

**依赖：** `docs/design/2026-07-12-runtime-memory-candidate-pipeline-v2.md`；实施 migration 前必须取得用户明确确认。

---

## 1. 文件结构

### 新建

- `backend/crates/golish-db/migrations/20260712000001_runtime_memory_foundation.sql`：additive runtime schema。
- `backend/crates/golish-db/src/repo/project_scopes.rs`：注册稳定 workspace security identity；显式 rename/retire。
- `backend/crates/golish-db/src/repo/operation_org_scope.rs`：冻结/读取 operation 公司快照。
- `backend/crates/golish-db/src/repo/stage_run_units.rs`：per-stage-execution/per-org final Gate unit 状态机；内部 wave 只映射到 WorkerRun work item。
- `backend/crates/golish-db/src/repo/stage_worker_runs.rs`：worker lease/chain/checkpoint CAS。
- `backend/crates/golish-db/src/repo/stage_handoffs.rs`：Gate PASS handoff。
- `backend/crates/golish-db/tests/runtime_memory_migrations.rs`：不依赖环境变量的 embedded PostgreSQL 空库/legacy upgrade migration test。
- `backend/crates/golish-db/tests/fixtures/runtime_memory_legacy_schema.sql`：升级测试所需的最小旧 schema 与 legacy state fixture。
- `backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs`：repo trait 的 PostgreSQL bridge。
- `backend/crates/golish-agent-app/src/ai/db_bridge/test_support.rs`：mandatory embedded bridge fixture（仅测试编译）。

### 修改

- `backend/crates/golish-db/src/repo/{mod.rs,tool_calls.rs,message_chains.rs,stage_runs.rs,stage_asset_waves.rs,tasks.rs}`
- `backend/crates/golish-agent-kit/src/db_traits/{types.rs,repo.rs}`
- `backend/crates/golish-agent-kit/src/db_shim.rs`
- `backend/crates/golish-agent-kit/src/task_orchestrator/{orchestrator.rs,harness_resume.rs,stage_execution.rs,types.rs,prompts/mod.rs}`
- `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/{execute.rs,execute_harness_loop_tests.rs}`
- `backend/crates/golish-agent-app/src/ai/db_bridge/mod.rs`
- `backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs`
- `backend/crates/golish-agent-kit/src/planner/tests/manager_tests.rs`
- `backend/crates/golish-agent-bridge/src/agent_bridge/{mod.rs,config.rs,prepare.rs,constructors/mod.rs}`
- `backend/crates/golish-agent-bridge/src/bridge_executor/trait_impl.rs`
- `backend/crates/golish-agent-runtime/src/agentic_loop/context.rs`
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/{stage_run_call.rs,sub_agent_call.rs}`
- `backend/crates/golish-sub-agents/src/executor_types.rs`
- `backend/crates/golish-sub-agents/src/executor/{chain_persist.rs,inner.rs}`
- `backend/crates/golish/src/stage_run/mod.rs`
- `backend/crates/golish-recon-app/src/organizations/{mod.rs,artifact_cleanup.rs}`
- `scripts/run_tree.py`
- `scripts/tests/test_run_tree_runtime_memory.py`
- 对应 `docs/modules/backend/**` 模块卡与 `docs/modules/INDEX.md`

---

## Task 0：执行仓库开工门禁并实现 rollout mode contract

**文件：** `backend/crates/golish-agent-bridge/src/agent_bridge/config.rs`、`backend/crates/golish-agent-bridge/src/agent_bridge/{mod.rs,prepare.rs}`、`backend/crates/golish-agent-runtime/src/agentic_loop/context.rs` 及其测试。

### 步骤 1：开工前置

1. 读 `agent-progress.md`、`feature_list.json`、本计划涉及的全部模块卡。
2. 当前 `in_progress` 不是本 feature 时先停止并请求用户决定，不自行抢槽。
3. 对 migration/`golish-db` 取得明确批准。
4. 运行 `./init.sh`，预期 exit 0；失败先修基础环境并记录，不进入 Task 1。
5. 运行 `git status --short`；任一 Task 将修改的文件若已含他人/用户未提交改动，先由用户 reconcile 或在独立干净 worktree 执行。禁止用整文件 staging 吞入不属于本 Task 的既有 hunk。

### 步骤 2：写 mode matrix RED

新增测试，实际调用 `RuntimeMemoryPolicy::decide`：

```rust
#[test]
fn runtime_memory_policy_covers_every_read_write_mode_pair() {
    use RuntimeMemoryReadMode::*;
    use RuntimeMemoryWriteMode::*;
    for read in [Legacy, PreferV2WithLegacyFallback, V2Only] {
        for write in [LegacyOnly, DualWrite, V2Only] {
            let decision = RuntimeMemoryPolicy::new(read, write).decide();
            assert_eq!(decision.read_source, expected_read_source(read));
            assert_eq!(decision.write_target, expected_write_target(write));
            assert!(!decision.may_merge_fields_from_two_sources);
        }
    }
}
```

RED 命令：

```bash
cd backend && cargo nextest run -p golish-agent-bridge runtime_memory_policy --no-tests=fail --status-level fail
```

预期：编译失败，缺少 mode/policy 类型。

### 步骤 3：实现 config 与传播

```rust
pub enum RuntimeMemoryReadMode { Legacy, PreferV2WithLegacyFallback, V2Only }
pub enum RuntimeMemoryWriteMode { LegacyOnly, DualWrite, V2Only }

pub struct RuntimeMemoryPolicy {
    pub read: RuntimeMemoryReadMode,
    pub write: RuntimeMemoryWriteMode,
}
```

默认 `Legacy/LegacyOnly`。`DualWrite` 只能调用 golish-db compound transaction：V2 rows 与 legacy JSON mirror 同事务成功或同事务回滚。read 每次选择整条 V2 worker 或整条 legacy checkpoint，禁止把两边字段拼成混合记录。operation 创建时冻结 `runtime_contract_version`，运行中不得切换。

### 步骤 4：GREEN

```bash
cd backend && cargo nextest run -p golish-agent-bridge runtime_memory_policy --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-runtime runtime_memory_policy --no-tests=fail --status-level fail
```

预期：所有 3×3 cases passed。

### 步骤 5：提交门禁

每个 Task 的后续“提交”步骤都必须先执行：

```bash
git diff --check
just precommit
git diff --cached --name-only
```

预期：前两条 exit 0；暂存区只含本 Task 在 `**文件：**` 中逐项列出的精确文件。禁止目录级 `git add`；若用户未授权 commit，只记录 checkpoint，不执行 `git commit`。

---

## Task 1：新增 additive schema 与纯状态机 repo

**文件：** 新建 migration 和五个 repo；修改 `repo/mod.rs`。

### 步骤 1：写失败的 schema contract 测试

在各 repo `#[cfg(test)]` 先声明 SQL contract 测试；测试至少包含：

```rust
#[test]
fn stage_worker_checkpoint_is_compare_and_swap() {
    assert!(CHECKPOINT_CAS_SQL.contains("checkpoint_version = checkpoint_version + 1"));
    assert!(CHECKPOINT_CAS_SQL.contains("lease_token = $"));
    assert!(CHECKPOINT_CAS_SQL.contains("attempt_epoch = $"));
    assert!(CHECKPOINT_CAS_SQL.contains("checkpoint_version = $"));
    assert!(CHECKPOINT_CAS_SQL.contains("stage_run_unit_id = $"));
    assert!(CLAIM_WORKER_SQL.contains("FOR UPDATE SKIP LOCKED"));
    assert!(CLAIM_WORKER_SQL.contains("lease_expires_at < NOW()"));
}

#[test]
fn operation_scope_membership_is_snapshot_scoped() {
    assert!(INSERT_SCOPE_HEADER_SQL.contains("project_scope_id"));
    assert!(INSERT_SCOPE_UNIT_SQL.contains("snapshot_id"));
    assert!(INSERT_SCOPE_UNIT_SQL.contains("organization_id"));
    assert!(!INSERT_SCOPE_UNIT_SQL.contains("WITH RECURSIVE"));
}

#[tokio::test]
async fn project_path_is_provenance_not_the_authorization_identity() {
    let fixture = RuntimeMemoryDbFixture::new().await;
    let first = fixture.resolve_project_scope("/workspace/original").await.unwrap();
    fixture.rename_project_path(first, "/workspace/renamed").await.unwrap();
    let reopened = fixture.resolve_existing_project_scope(first, "/workspace/renamed").await.unwrap();
    assert_eq!(reopened.project_scope_id, first);
    assert_eq!(reopened.canonical_project_path, "/workspace/renamed");
}
```

运行：

```bash
cd backend && cargo nextest run -p golish-db -E 'test(operation_scope) | test(stage_run_unit) | test(stage_worker) | test(stage_handoff)' --no-tests=fail --status-level fail
```

预期：编译失败，报告新模块/常量不存在。

### 步骤 2：写 migration

创建以下表和约束：

```sql
CREATE TABLE project_scopes (
    project_scope_id UUID PRIMARY KEY,
    canonical_project_path TEXT NOT NULL UNIQUE,
    row_version BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    retired_at TIMESTAMPTZ
);

CREATE TABLE operation_org_scope_snapshots (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL UNIQUE REFERENCES operation_state(operation_id) ON DELETE CASCADE,
    project_scope_id UUID NOT NULL REFERENCES project_scopes(project_scope_id) ON DELETE RESTRICT,
    project_path_at_freeze TEXT NOT NULL,
    root_organization_id UUID NOT NULL,
    decision_tool_call_id UUID REFERENCES tool_calls(id) ON DELETE SET NULL,
    scope_hash TEXT NOT NULL,
    frozen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    invalidated_at TIMESTAMPTZ,
    invalidation_reason TEXT,
    schema_version INTEGER NOT NULL DEFAULT 1,
    UNIQUE(id, operation_id)
);

ALTER TABLE stage_runs
    ADD CONSTRAINT stage_runs_id_operation_unique UNIQUE(id, operation_id);

CREATE TABLE operation_org_scope_units (
    snapshot_id UUID NOT NULL REFERENCES operation_org_scope_snapshots(id) ON DELETE CASCADE,
    organization_id UUID NOT NULL,
    parent_organization_id UUID,
    organization_name TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('root','subsidiary')),
    depth INTEGER NOT NULL CHECK (depth >= 0),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    ownership_percent NUMERIC,
    approval_source JSONB NOT NULL,
    PRIMARY KEY (snapshot_id, organization_id),
    UNIQUE (snapshot_id, ordinal)
);

CREATE TABLE stage_run_units (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE CASCADE,
    stage_execution_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL REFERENCES operation_org_scope_snapshots(id),
    organization_id UUID NOT NULL,
    stage_kind TEXT NOT NULL,
    generation INTEGER NOT NULL CHECK (generation >= 0),
    specialist TEXT,
    status TEXT NOT NULL CHECK (status IN ('queued','running','gate_blocked','passed','exhausted','superseded')),
    gate_attempt INTEGER NOT NULL DEFAULT 0,
    pass_watermark JSONB NOT NULL DEFAULT '{}',
    row_version BIGINT NOT NULL DEFAULT 0,
    started_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    terminal_at TIMESTAMPTZ,
    UNIQUE (stage_execution_id, organization_id),
    FOREIGN KEY (stage_execution_id, operation_id)
        REFERENCES stage_runs(id, operation_id),
    FOREIGN KEY (scope_snapshot_id, operation_id)
        REFERENCES operation_org_scope_snapshots(id, operation_id),
    FOREIGN KEY (scope_snapshot_id, organization_id)
        REFERENCES operation_org_scope_units(snapshot_id, organization_id)
);

CREATE TABLE stage_worker_runs (
    id UUID PRIMARY KEY,
    stage_run_unit_id UUID NOT NULL REFERENCES stage_run_units(id) ON DELETE CASCADE,
    worker_generation INTEGER NOT NULL,
    specialist TEXT NOT NULL,
    work_item_kind TEXT NOT NULL,
    work_item_key TEXT NOT NULL,
    agent_path TEXT NOT NULL,
    parent_request_id TEXT,
    message_chain_id UUID REFERENCES message_chains(id) ON DELETE SET NULL,
    status TEXT NOT NULL CHECK (status IN ('queued','running','waiting_background','gate_blocked','passed','failed','exhausted','superseded')),
    gate_attempt INTEGER NOT NULL DEFAULT 0,
    checkpoint JSONB NOT NULL DEFAULT '{}',
    checkpoint_version BIGINT NOT NULL DEFAULT 0,
    lease_token UUID,
    lease_owner TEXT,
    lease_acquired_at TIMESTAMPTZ,
    lease_expires_at TIMESTAMPTZ,
    heartbeat_at TIMESTAMPTZ,
    attempt_epoch BIGINT NOT NULL DEFAULT 0,
    evidence_watermark BIGINT,
    started_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    terminal_at TIMESTAMPTZ,
    CHECK (
      (lease_token IS NULL AND lease_owner IS NULL AND lease_expires_at IS NULL)
      OR (lease_token IS NOT NULL AND lease_owner IS NOT NULL AND lease_expires_at IS NOT NULL)
    ),
    UNIQUE(stage_run_unit_id, work_item_kind, work_item_key, worker_generation)
);

CREATE UNIQUE INDEX stage_worker_runs_chain_owner
ON stage_worker_runs(message_chain_id)
WHERE message_chain_id IS NOT NULL;

CREATE TABLE stage_handoffs (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE CASCADE,
    organization_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    from_stage_kind TEXT NOT NULL,
    stage_execution_id UUID NOT NULL,
    source_stage_run_unit_id UUID NOT NULL UNIQUE REFERENCES stage_run_units(id),
    scope_hash TEXT NOT NULL,
    payload JSONB NOT NULL,
    evidence_ids BIGINT[] NOT NULL DEFAULT '{}',
    coverage_watermark JSONB NOT NULL DEFAULT '{}',
    unit_gate_decision_hash TEXT NOT NULL,
    aggregate_pass_token_hash TEXT,
    gate_passed_at TIMESTAMPTZ NOT NULL,
    invalidated_at TIMESTAMPTZ,
    schema_version INTEGER NOT NULL DEFAULT 1,
    UNIQUE(stage_execution_id, organization_id),
    FOREIGN KEY (scope_snapshot_id, organization_id)
        REFERENCES operation_org_scope_units(snapshot_id, organization_id),
    FOREIGN KEY (scope_snapshot_id, operation_id)
        REFERENCES operation_org_scope_snapshots(id, operation_id),
    FOREIGN KEY (stage_execution_id, operation_id)
        REFERENCES stage_runs(id, operation_id)
);
```

### 步骤 3：实现 repo 类型和 transition

使用明确状态枚举与 CAS：

```rust
pub enum StageRunUnitStatus {
    Queued,
    Running,
    GateBlocked,
    Passed,
    Exhausted,
    Superseded,
}

pub fn can_transition_unit(from: StageRunUnitStatus, to: StageRunUnitStatus) -> bool {
    matches!(
        (from, to),
        (Queued, Running)
            | (Running, GateBlocked)
            | (Running, Passed)
            | (Running, Exhausted)
            | (Running, Superseded)
            | (GateBlocked, Running)
            | (GateBlocked, Exhausted)
            | (GateBlocked, Superseded)
    )
}

pub async fn checkpoint_cas(
    pool: &PgPool,
    worker_run_id: Uuid,
    stage_run_unit_id: Uuid,
    lease_token: Uuid,
    attempt_epoch: i64,
    checkpoint: &Value,
    expected_version: i64,
) -> Result<i64>;
```

`operation_org_scope::freeze` 必须从 trusted workspace registration 取得稳定 `project_scope_id`，再在单事务中插 header + units。首次打开 workspace 时可按 canonical path 注册；目录移动/rename 必须由可信 UI/CLI 带 `expected_project_scope_id` 显式更新 path，不能仅凭一个新路径猜测为旧项目，也不能静默生成第二个 security scope id。snapshot 冲突时读取既有值并比较 hash，hash 不同返回 conflict，不覆盖。`scope_hash` 固定为 SHA-256：对 `schema_version|project_scope_id|project_path_at_freeze|root_organization_id|mode|units` 做 UTF-8 canonical JSON，units 按 `(depth, ordinal, organization_id)` 排序，每项只含 org id、parent id、role、ownership percent 和 approval decision id。authorization/RAG 只用 `project_scope_id`，path 只作冻结时 provenance。

`project_scopes.rs` 提供 `register_first_open(canonical_path)`、`rename(expected_project_scope_id, expected_old_path, new_path)` 和 `retire(expected_project_scope_id)`；rename/retire 都用 row-version CAS，旧 path 或 ID 不匹配返回 conflict。operation/session context 保存 project_scope_id，不能让模型或普通 tool args 覆写。

unit transition 使用 `UPDATE ... WHERE status=$expected AND row_version=$expected_version`；`gate_attempt` 只在 `GateBlocked→Running` 加一，terminal transition 同时写 terminal_at。generation 由 `transition_stage_execution` transaction 原子分配，不由 runtime 自行 `max+1`。

`stage_worker_runs` repo 必须提供 `claim_lease/heartbeat/release`；只有 queued、gate_blocked 或 lease 已过期的 nonterminal worker 可 claim。所有 checkpoint/terminal SQL 的 WHERE 同时包含 worker id、stage unit id、lease token、attempt epoch、expected version 和允许的旧状态。

`stage_handoffs::publish_gate_passed` 只接受 final-sealed `StageRunUnitStatus::Passed` 的 source unit，payload 由服务器根据 GateContext 构造，并逐项验证 operation/org/snapshot/hash、canonical ref current/fresh、evidence ownership。中间 wave close 不发布 Handoff。

### 步骤 4：实现强制 embedded migration tests

`runtime_memory_migrations.rs` 不读取 `GOLISH_TEST_DATABASE_URL`，也不允许 early return：

```rust
fn reserve_local_port() -> std::io::Result<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}

#[tokio::test]
#[serial_test::serial]
async fn runtime_memory_migration_upgrades_empty_and_legacy_fixtures() {
    for with_legacy_rows in [false, true] {
        let data = tempfile::tempdir().expect("temp pgdata");
        let port = reserve_local_port().expect("free local port");
        let mut config = DbConfig::default();
        config.pg_data_dir = data.path().join("pgdata");
        config.port = port;
        config.database = format!("runtime_memory_{with_legacy_rows}");
        let mut embedded = EmbeddedPg::start(config.clone()).await.expect("embedded pg");
        let pool = PgPoolOptions::new().connect(&config.connection_string()).await.expect("pool");
        sqlx::raw_sql(include_str!("fixtures/runtime_memory_legacy_schema.sql"))
            .execute(&pool).await.expect("legacy schema");
        if with_legacy_rows {
            sqlx::raw_sql(r#"
                INSERT INTO organizations(id, project_path, name)
                VALUES ('10000000-0000-0000-0000-000000000001', '/fixture', 'Root');
                INSERT INTO operation_state(operation_id, profile, current_stage, engagement_org_id, state_blob)
                VALUES (
                  '20000000-0000-0000-0000-000000000001', 'pentest', 'enumeration',
                  '10000000-0000-0000-0000-000000000001',
                  '{"stage_run_workers":{"enumeration":{"10000000-0000-0000-0000-000000000001":{"chain_id":"30000000-0000-0000-0000-000000000001"}}}}'
                );
            "#).execute(&pool).await.expect("legacy rows");
        }
        sqlx::raw_sql(include_str!("../migrations/20260712000001_runtime_memory_foundation.sql"))
            .execute(&pool).await.expect("runtime migration");
        for table in [
            "operation_org_scope_snapshots",
            "operation_org_scope_units",
            "stage_run_units",
            "stage_worker_runs",
            "stage_handoffs",
        ] {
            let exists: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
                .bind(table).fetch_one(&pool).await.expect("table lookup");
            assert!(exists, "{table} must exist");
        }
        if with_legacy_rows {
            let blob: serde_json::Value = sqlx::query_scalar(
                "SELECT state_blob FROM operation_state WHERE operation_id='20000000-0000-0000-0000-000000000001'"
            ).fetch_one(&pool).await.expect("legacy blob");
            assert_eq!(blob["stage_run_workers"]["enumeration"].as_object().map(|v| v.len()), Some(1));
        }
        embedded.stop().await;
    }
}
```

fixture 定义 migration 引用的最小 old schema；helper 对 unique/composite FK/check/lease columns 执行真实 INSERT/UPDATE hostile fixtures，不做 SQL substring-only false green。

### 步骤 5：运行 GREEN

```bash
cd backend && cargo nextest run -p golish-db -E 'test(operation_scope) | test(stage_run_unit) | test(stage_worker) | test(stage_handoff) | test(runtime_memory_migration)' --no-tests=fail --status-level fail
```

预期：新测试全部 passed；无 ignored failure。

### 步骤 6：提交

```bash
just precommit
git add -- backend/crates/golish-db/migrations/20260712000001_runtime_memory_foundation.sql backend/crates/golish-db/src/repo/project_scopes.rs backend/crates/golish-db/src/repo/operation_org_scope.rs backend/crates/golish-db/src/repo/stage_run_units.rs backend/crates/golish-db/src/repo/stage_worker_runs.rs backend/crates/golish-db/src/repo/stage_handoffs.rs backend/crates/golish-db/src/repo/mod.rs backend/crates/golish-db/tests/runtime_memory_migrations.rs backend/crates/golish-db/tests/fixtures/runtime_memory_legacy_schema.sql
git diff --cached --name-only
git commit -m "feat(db): add operation runtime memory foundation"
```

---

## Task 2：扩展 repo trait 与 PostgreSQL bridge

**文件：** `db_traits/{types.rs,repo.rs}`、`db_shim.rs`、`db_bridge/{mod.rs,runtime_memory.rs}`。

### 步骤 1：写 mock trait 失败测试

在 `runtime_memory.rs` 新增 mandatory embedded bridge 测试，复用 Task 1 test harness，不允许因环境变量缺失跳过：

```rust
#[tokio::test]
async fn runtime_memory_bridge_roundtrips_org_isolated_rows() {
    let fixture = RuntimeMemoryDbFixture::start().await;
    let bridge = PgDbBridge::new(fixture.pool().clone());
    let project_scope = bridge.project_scope_register(
        RegisterProjectScope::first_open(fixture.project_path())
    ).await.expect("project scope");
    let snapshot = bridge.operation_org_scope_freeze(
        FreezeOperationScope::root_only(fixture.operation_id(), project_scope.project_scope_id, fixture.project_path(), fixture.root_org_id())
    ).await.expect("freeze");
    let units = bridge.stage_run_units_seed(
        SeedStageRunUnits::single_org(fixture.stage_execution_id(), snapshot.id, fixture.root_org_id())
    ).await.expect("seed");
    let worker = bridge.stage_worker_run_claim(
        ClaimWorkerRun::organization(units[0].id, fixture.root_org_id(), "enumerator", "test-worker")
    ).await.expect("worker");
    assert_eq!(worker.stage_run_unit_id, units[0].id);
    assert_eq!(worker.work_item_key, fixture.root_org_id().to_string());
}

#[tokio::test]
async fn runtime_memory_bridge_rejects_sibling_org_transition() {
    let fixture = RuntimeMemoryDbFixture::start().await;
    let bridge = PgDbBridge::new(fixture.pool().clone());
    let err = bridge.stage_run_units_seed(
        SeedStageRunUnits::single_org(
            fixture.stage_execution_id(),
            fixture.root_only_snapshot_id(),
            fixture.sibling_org_id(),
        )
    ).await.expect_err("sibling outside snapshot must fail");
    assert!(matches!(err, RuntimeMemoryError::OrganizationOutsideSnapshot { .. }));
}
```

运行：

```bash
cd backend && cargo nextest run -p golish-agent-app runtime_memory_bridge --no-tests=fail --status-level fail
```

预期：编译失败，DTO/trait methods 不存在。

### 步骤 2：新增 DTO

在 `db_traits/types.rs` 定义：

```rust
pub struct ProjectScopeView { pub project_scope_id: Uuid, pub canonical_project_path: String, pub retired_at: Option<DateTime<Utc>> }
pub struct OperationOrgScopeSnapshotView { pub id: Uuid, pub operation_id: Uuid, pub project_scope_id: Uuid, pub project_path_at_freeze: String, pub scope_hash: String, pub invalidated_at: Option<DateTime<Utc>>, pub units: Vec<OperationOrgScopeUnitView> }
pub struct OperationOrgScopeUnitView { pub organization_id: Uuid, pub parent_organization_id: Option<Uuid>, pub organization_name: String, pub role: String, pub depth: i32, pub ordinal: i32 }
pub struct StageRunUnitView { pub id: Uuid, pub operation_id: Uuid, pub stage_execution_id: Uuid, pub organization_id: Uuid, pub stage_kind: String, pub generation: i32, pub status: String }
pub struct StageWorkerRunView { pub id: Uuid, pub stage_run_unit_id: Uuid, pub work_item_kind: String, pub work_item_key: String, pub message_chain_id: Option<Uuid>, pub status: String, pub checkpoint: Value, pub checkpoint_version: i64, pub lease_token: Option<Uuid>, pub attempt_epoch: i64 }
pub struct StageHandoffView { pub operation_id: Uuid, pub organization_id: Uuid, pub from_stage_kind: String, pub stage_execution_id: Uuid, pub scope_hash: String, pub unit_gate_decision_hash: String, pub aggregate_pass_token_hash: Option<String>, pub payload: Value, pub evidence_ids: Vec<i64> }
```

### 步骤 3：新增 fail-closed trait methods

方法必须返回 `Result`，不能有默认空 Vec 兜底：

```rust
async fn project_scope_register(&self, command: RegisterProjectScope) -> anyhow::Result<ProjectScopeView>;
async fn project_scope_rename(&self, command: RenameProjectScope) -> anyhow::Result<ProjectScopeView>;
async fn operation_org_scope_get(&self, operation_id: Uuid) -> anyhow::Result<Option<OperationOrgScopeSnapshotView>>;
async fn operation_org_scope_freeze(&self, command: FreezeOperationScope) -> anyhow::Result<OperationOrgScopeSnapshotView>;
async fn stage_run_units_seed(&self, command: SeedStageRunUnits) -> anyhow::Result<Vec<StageRunUnitView>>;
async fn stage_execution_transition(&self, command: TransitionStageExecution) -> anyhow::Result<StageExecutionTransitionResult>;
async fn stage_worker_run_claim(&self, command: ClaimWorkerRun) -> anyhow::Result<StageWorkerRunView>;
async fn stage_worker_run_heartbeat(&self, command: HeartbeatWorkerRun) -> anyhow::Result<StageWorkerRunView>;
async fn stage_worker_run_create_bound_chain(&self, command: CreateBoundWorkerChain) -> anyhow::Result<StageWorkerRunView>;
async fn stage_worker_run_checkpoint_cas(&self, command: WorkerCheckpointCas) -> anyhow::Result<StageWorkerRunView>;
async fn stage_handoffs_publish(&self, command: PublishStageHandoff) -> anyhow::Result<StageHandoffView>;
async fn stage_handoffs_list_inherited(&self, query: InheritedHandoffQuery) -> anyhow::Result<Vec<StageHandoffView>>;
```

这些 methods 不提供返回空集合的默认实现；同步更新 production `GolishDbRepoProvider`、`planner/tests/manager_tests.rs::StubRepo` 和 `execute_harness_loop_tests.rs::MemRepo`。两个 test repo 对未配置 V2 state 返回 `Err(RuntimeMemoryUnsupported)`，V2 测试必须显式配置 fixture。

### 步骤 4：实现 bridge 并 GREEN

`runtime_memory.rs` 只负责 row→view 和 repo delegate，不复制业务状态机。

```bash
cd backend && cargo nextest run -p golish-agent-app runtime_memory_bridge --no-tests=fail --status-level fail
cd backend && cargo check -p golish-agent-app
```

预期：测试 passed，check exit 0。

### 步骤 5：提交

```bash
just precommit
git add -- backend/crates/golish-agent-kit/src/db_traits/types.rs backend/crates/golish-agent-kit/src/db_traits/repo.rs backend/crates/golish-agent-kit/src/db_shim.rs backend/crates/golish-agent-kit/src/planner/tests/manager_tests.rs backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute_harness_loop_tests.rs backend/crates/golish-agent-app/src/ai/db_bridge/mod.rs backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs backend/crates/golish-agent-app/src/ai/db_bridge/test_support.rs
git diff --cached --name-only
git commit -m "feat(harness): expose runtime memory repository contract"
```

---

## Task 3：修正 stage_run identity 生命周期

**文件：** orchestrator、harness resume、stage execution、bridge/runtime context。

### 步骤 1：写 RED

新增：

```rust
#[tokio::test]
async fn same_stage_resume_reuses_active_stage_execution_id() {
    let mut repo = MemRepo::with_operation(StageKind::Enumeration);
    let first = repo.stage_execution_transition(
        TransitionStageExecution::enter(repo.operation_id(), StageKind::Enumeration)
    ).await.expect("first entry");
    let resumed = repo.stage_execution_transition(
        TransitionStageExecution::enter(repo.operation_id(), StageKind::Enumeration)
    ).await.expect("resume");
    assert_eq!(first.stage_execution_id, resumed.stage_execution_id);
    assert_eq!(resumed.outcome, StageExecutionTransitionOutcome::Reused);
}

#[tokio::test]
async fn stage_transition_terminalizes_previous_and_creates_next_execution() {
    let mut repo = MemRepo::with_operation(StageKind::Enumeration);
    let previous = repo.stage_execution_transition(
        TransitionStageExecution::enter(repo.operation_id(), StageKind::Enumeration)
    ).await.expect("enumeration");
    let next = repo.stage_execution_transition(
        TransitionStageExecution::advance(repo.operation_id(), previous.stage_execution_id, StageKind::VulnTriage)
    ).await.expect("advance");
    assert_ne!(previous.stage_execution_id, next.stage_execution_id);
    assert_eq!(repo.execution_status(previous.stage_execution_id), Some("completed"));
    assert_eq!(repo.operation_stage(), StageKind::VulnTriage);
}
```

```bash
cd backend && cargo nextest run -p golish-agent-kit -E 'test(same_stage_resume) | test(stage_transition_terminalizes)' --no-tests=fail --status-level fail
```

预期：旧路径只在初始 operation 创建 stage_run，测试失败。

### 步骤 2：传播 trusted id

给 `ExecutionContext`、`AgentBridge` side-channel、`AgenticLoopContext` 增：

```rust
pub harness_stage_execution_id: Option<Uuid>
```

在 `golish-db/src/repo/stage_runs.rs` 实现 compound `transition_stage_execution`：同 stage/current id 复用；advance 在一笔短事务中锁 operation_state、CAS terminal previous、insert next、advance operation cursor，返回 `Reused|Created`。paused/failed/completed 使用显式状态表，重放相同 command 幂等，冲突 stage/id 返回 error。

`StageDeliverable.stage_run_id` 保留 wire 兼容名，但 `harness_submit_tool.rs` 不再生成 fresh UUID，而是覆盖为 trusted active `stage_execution_id`；当前 submit 的 `tool_call_id` 是独立 `deliverable_submission_id`。同一 stage 多次 submit 的 execution id 必须相同。

### 步骤 3：更新所有 test/eval context 构造

在 `golish-agent-runtime/src/eval_support/*` 和 `test_utils/context.rs` 明确填 `None`，不使用结构体更新语法掩盖缺字段。

### 步骤 4：GREEN

```bash
cd backend && cargo nextest run -p golish-agent-kit -E 'test(same_stage_resume) | test(stage_transition_terminalizes) | test(multiple_submissions_keep_stage_execution_id)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-bridge bridge_executor --no-tests=fail --status-level fail
cd backend && cargo check -p golish-agent-runtime
```

### 步骤 5：提交

```bash
just precommit
git add -- backend/crates/golish-db/src/repo/stage_runs.rs backend/crates/golish-agent-kit/src/task_orchestrator/orchestrator.rs backend/crates/golish-agent-kit/src/task_orchestrator/harness_resume.rs backend/crates/golish-agent-kit/src/task_orchestrator/stage_execution.rs backend/crates/golish-agent-kit/src/task_orchestrator/types.rs backend/crates/golish-agent-bridge/src/agent_bridge/mod.rs backend/crates/golish-agent-bridge/src/agent_bridge/config.rs backend/crates/golish-agent-bridge/src/agent_bridge/prepare.rs backend/crates/golish-agent-bridge/src/agent_bridge/constructors/mod.rs backend/crates/golish-agent-bridge/src/bridge_executor/trait_impl.rs backend/crates/golish-agent-runtime/src/agentic_loop/context.rs backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs
git diff --cached --name-only
git commit -m "fix(harness): make stage run identity durable across resume"
```

---

## Task 4：在 Scoping PASS 后冻结 org scope，并统一 CLI/chat

**文件：** `tool_calls.rs`、`execute.rs`、`golish/src/stage_run/mod.rs`、runtime stage_run。

### 步骤 1：写 RED

测试必须覆盖：

```rust
#[tokio::test]
async fn freeze_uses_the_persisted_decision_not_the_mutable_tree() {
    let mut fixture = ScopeFixture::root_with_children(["Approved Child", "Unapproved Child"]);
    fixture.record_included_review(["Approved Child"]).await;
    let snapshot = freeze_from_persisted_review(
        fixture.pool(), fixture.operation_id(), fixture.task_id(), fixture.stage_execution_id(), fixture.root_id()
    ).await.expect("freeze included review");
    assert_eq!(snapshot.organization_ids(), vec![fixture.root_id(), fixture.org_id("Approved Child")]);
    fixture.insert_child("Late Child").await;
    let reread = get_for_operation(fixture.pool(), fixture.operation_id()).await.expect("reread").unwrap();
    assert_eq!(reread.scope_hash, snapshot.scope_hash);
    assert!(!reread.organization_ids().contains(&fixture.org_id("Late Child")));
}

#[tokio::test]
async fn root_only_choice_needs_no_unit_review_and_freeze_failure_stops_transition() {
    let mut fixture = ScopeFixture::root_with_children(["Child"]);
    fixture.record_root_only_choice().await;
    let snapshot = freeze_from_persisted_review(
        fixture.pool(), fixture.operation_id(), fixture.task_id(), fixture.stage_execution_id(), fixture.root_id()
    ).await.expect("root-only freeze");
    assert_eq!(snapshot.organization_ids(), vec![fixture.root_id()]);
    fixture.repo().fail_next_scope_freeze();
    let outcome = fixture.orchestrator().advance_after_scoping_pass().await;
    assert!(matches!(outcome, StageOutcome::Blocked { code: "scope_freeze_failed", .. }));
    assert_eq!(fixture.operation_stage(), StageKind::Scoping);
}

#[test]
fn cli_subsidiary_flags_freeze_one_operation_snapshot() {
    let selection = resolve_cli_scope_once(CliScopeInput::include_descendants(
        root(), vec![child_60_percent(), grandchild_80_percent(), child_40_percent()], 51.0
    )).expect("CLI scope");
    assert_eq!(selection.operation_count, 1);
    assert_eq!(selection.units, vec![root(), child_60_percent(), grandchild_80_percent()]);
    assert_eq!(selection.approval_source, "cli_flags");
}
```

运行对应 `operation_scope`/`scope_freeze` filters，预期旧 live subtree 行为使测试失败。

### 步骤 2：返回结构化 review decision

在 `tool_calls.rs` 新增：

```rust
pub struct ApprovedOrgScopeDecision {
    pub decision_tool_call_id: Uuid,
    pub operation_id: Uuid,
    pub task_id: Uuid,
    pub stage_execution_id: Uuid,
    pub root_organization_id: Uuid,
    pub mode: ApprovedOrgScopeMode,
    pub approved_units: Vec<ApprovedOrgUnit>,
}

pub enum ApprovedOrgScopeMode {
    RootOnly { choice_tool_call_id: Uuid },
    Included { choice_tool_call_id: Uuid, proposal_tool_call_id: Uuid, review_tool_call_id: Uuid },
    ReuseReconfirmed { review_tool_call_id: Uuid },
    CliFlags { include_subsidiaries: bool, threshold_percent: Option<f64> },
}
```

reader 以 exact `operation_id + task_id + stage_execution_id + trusted root` 查询：root-only 读取结构化 `subsidiary_scope` choice且不要求 unit_review；included 读取同 epoch 的 choice→proposal→non-skipped unit_review；REUSE 要求本 operation reconfirm；CLI 使用 trusted flags 和一次性解析结果。拒绝旧 epoch、foreign same-session、unknown/cross-project child，不再只返回 bool。

### 步骤 3：Scoping PASS 原子 freeze

`execute.rs` 在 advance 前调用 `operation_org_scope_freeze`；失败把 outcome 改为 BLOCK/error，不写进下游 handoff。

组织删除前：若 scope 中有 active operation/worker 或未终态 cleanup obligation，删除返回带 `code` 的 conflict；可删除时先写 `OrganizationDeleted` invalidation event、终止 lease/失效 projection，再删除 live organization。scope snapshot/unit 不保留 live FK，继续保存 id/name-at-freeze；audit/evidence 永不 cascade。

### 步骤 4：替换 live subtree fanout

`stage_run_call.rs` 从：

```rust
repo.org_subtree_units(root)
```

改为：

```rust
repo.operation_org_scope_get(operation_id)
```

模型 tool args 只用于诊断；不能增加 snapshot 外 org。snapshot 缺失在 V2Only 模式 fail closed。

### 步骤 5：GREEN 与提交

```bash
cd backend && cargo nextest run -p golish-db operation_scope --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-kit scope_freeze --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-runtime stage_run_scope --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish stage_run_scope --no-tests=fail --status-level fail
```

预期全部 passed。

```bash
just precommit
git add -- backend/crates/golish-db/src/repo/tool_calls.rs backend/crates/golish-db/src/repo/operation_org_scope.rs backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute_harness_loop_tests.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs backend/crates/golish/src/stage_run/mod.rs backend/crates/golish-recon-app/src/organizations/mod.rs backend/crates/golish-recon-app/src/organizations/artifact_cleanup.rs
git diff --cached --name-only
git commit -m "feat(scope): freeze operation organization membership"
```

---

## Task 5：把 per-org worker 主写路径迁到 StageRunUnit/WorkerRun

**文件：** `stage_run_call.rs`、`sub_agent_call.rs`、sub-agent chain persistence。

### 步骤 1：写 RED

```rust
#[tokio::test]
async fn two_org_workers_have_independent_leases_and_checkpoint_cas() {
    let fixture = RuntimeFixture::two_org_stage_units().await;
    let a = fixture.claim_org_worker(fixture.org_a(), "owner-a").await.unwrap();
    let b = fixture.claim_org_worker(fixture.org_b(), "owner-b").await.unwrap();
    let a2 = fixture.checkpoint(a.id, a.lease_token.unwrap(), a.attempt_epoch, 0, json!({"turn": 1})).await.unwrap();
    assert_eq!(a2.checkpoint_version, 1);
    assert_eq!(fixture.load_worker(b.id).await.checkpoint, json!({}));
    let stale = fixture.checkpoint(a.id, a.lease_token.unwrap(), a.attempt_epoch, 0, json!({"turn": 2})).await;
    assert!(matches!(stale, Err(RuntimeMemoryError::StaleCheckpoint { .. })));
    let duplicate = fixture.claim_existing(a.id, "owner-c").await;
    assert!(matches!(duplicate, Err(RuntimeMemoryError::LeaseHeld { .. })));
}

#[tokio::test]
async fn bound_chain_survives_every_pre_provider_crash_window() {
    for crash in [CrashPoint::AfterChainInsert, CrashPoint::AfterInitialCheckpoint, CrashPoint::BeforeProvider] {
        let fixture = RuntimeFixture::single_worker_with_failure_injection(crash).await;
        let _ = fixture.dispatch_worker().await;
        let worker = fixture.reload_worker().await;
        let chain_id = worker.message_chain_id.expect("chain must be atomically bound");
        assert_eq!(fixture.chain_owner(chain_id).await, Some(worker.id));
        assert_eq!(fixture.provider_call_count(), 0);
        assert!(fixture.resume_exact_worker().await.is_ok());
    }
}

#[tokio::test]
async fn worker_result_uses_its_local_deliverable_not_shared_sink() {
    let (org_a, org_b) = run_two_workers_with_distinct_responses("A-deliverable", "B-deliverable").await;
    assert_eq!(org_a.parsed_response, "A-deliverable");
    assert_eq!(org_b.parsed_response, "B-deliverable");
}
```

### 步骤 2：seed/claim durable units

在 `stage_run` 开始时：

```rust
let units = repo.stage_run_units_seed(SeedStageRunUnits {
    operation_id,
    stage_execution_id,
    scope_snapshot_id,
    stage_kind,
    generation,
    organizations,
}).await?;
```

每个 stage execution/org 只 seed 一个 unit。逐 unit claim，已 `passed` 的直接跳过；`gate_blocked` resume 同 unit 并递增 gate_attempt。new asset wave 创建 `work_item_kind="asset_wave"/work_item_key=<wave_id>` 的新 WorkerRun，不创建新的 StageRunUnit；只有新 stage execution 才分配新 generation。

### 步骤 3：原子创建并绑定 chain

在 `golish-db/src/repo/message_chains.rs` 新增：

```rust
pub async fn create_bound_to_worker(
    tx: &mut Transaction<'_, Postgres>,
    chain_id: Uuid,
    worker_run_id: Uuid,
    lease_token: Uuid,
    attempt_epoch: i64,
    new_chain: NewMessageChain,
) -> Result<()>;
```

该函数在一笔事务中用预分配 id insert `message_chains` 并 CAS bind WorkerRun。事务提交后 executor 才写 initial provider-safe chain；`SubAgentChainCheckpointHook` 只回报后续 checkpoint watermark，不承担首次 bind。restore 校验 chain 唯一 owner、lease 和 exact work item；任何校验失败不调用 provider。ordinary non-stage subagent 保留现有 create path。

### 步骤 4：使用 local result

per-org deliverable 从当前 `ToolExecutionResult.value.response` 解析；共享 `RwLock<Option<String>>` 仅保留 legacy non-stage-run capture。

### 步骤 5：移除 V2 write 对 legacy JSON 的依赖

V2 写路径不调用：

- `upsert_stage_run_worker_blob`
- `state_blob_with_agent_run`

`V2Only` 不写上述 legacy slots。`DualWrite` 只能走 Task 0 的 compound DB transaction，同时写 V2 与 legacy mirror；`PreferV2WithLegacyFallback` 每次选择完整来源并在 trace 标记 `legacy_runtime_memory=true`，不能字段级混读。

### 步骤 6：GREEN 与提交

```bash
cd backend && cargo nextest run -p golish-agent-runtime -E 'test(two_org_workers) | test(bound_chain) | test(worker_result_uses)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-sub-agents -E 'test(provider_request_waits) | test(chain)' --no-tests=fail --status-level fail
```

```bash
just precommit
git add -- backend/crates/golish-db/src/repo/message_chains.rs backend/crates/golish-db/src/repo/stage_worker_runs.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs backend/crates/golish-sub-agents/src/executor_types.rs backend/crates/golish-sub-agents/src/executor/chain_persist.rs backend/crates/golish-sub-agents/src/executor/inner.rs
git diff --cached --name-only
git commit -m "feat(runtime): persist one checkpoint per stage worker"
```

---

## Task 6：发布 durable per-org StageHandoff

**文件：** `execute.rs`、`stage_run_call.rs`、`prompts/mod.rs`、bridge/repo。

### 步骤 1：写 RED

```rust
#[tokio::test]
async fn handoff_requires_final_pass_and_valid_same_org_sources() {
    let fixture = HandoffFixture::blocked_unit().await;
    let blocked = fixture.publish_server_built_handoff().await;
    assert!(matches!(blocked, Err(RuntimeMemoryError::UnitNotFinalPassed { .. })));
    fixture.mark_final_passed().await;
    fixture.inject_foreign_org_evidence();
    let foreign = fixture.publish_server_built_handoff().await;
    assert!(matches!(foreign, Err(RuntimeMemoryError::ForeignEvidence { .. })));
}

#[tokio::test]
async fn gate_pass_handoff_is_atomic_and_restart_reads_only_exact_org() {
    let fixture = HandoffFixture::two_orgs().await;
    fixture.fail_after_unit_pass_before_handoff_insert();
    assert!(fixture.finalize_org_a().await.is_err());
    assert_eq!(fixture.unit_status(fixture.org_a()).await, StageRunUnitStatus::Running);
    assert!(fixture.handoffs(fixture.org_a()).await.is_empty());
    fixture.clear_failure();
    fixture.finalize_org_a().await.expect("atomic final pass");
    let resumed = fixture.restart_and_load_inherited(fixture.org_a()).await;
    assert_eq!(resumed.len(), 1);
    assert_eq!(resumed[0].organization_id, fixture.org_a());
    assert!(fixture.restart_and_load_inherited(fixture.org_b()).await.is_empty());
}
```

### 步骤 2：定义 bounded payload

```rust
pub struct StageHandoffPayload {
    pub canonical_fact_refs: Vec<CanonicalFactRef>,
    pub typed_claims: Vec<TypedHandoffClaim>,
    pub coverage_watermark: Value,
    pub evidence_ids: Vec<i64>,
}
```

payload 没有 public/model-controlled constructor；`build_stage_handoff_from_gate_context` 读取 GateContext、canonical repo 和 evidence ledger生成。每个 Vec 有固定上限；超限返回 explicit error，不静默截断 evidence identity。

### 步骤 3：PASS 后事务发布

per-org final Gate transition 和 handoff insert 在 repo 短事务完成；中间 wave 只更新 worklist/watermark。handoff 写 `unit_gate_decision_hash`，aggregate stage pass token 出现后可补 `aggregate_pass_token_hash`，但不是 per-org publish 前置。非-specialist operation stage 也使用 root unit 的 final Gate decision。

### 步骤 4：按 StageSpec 读取

stage entry 按 `inherits_evidence_from` 查询同 operation/org 的 latest final-sealed execution；prompt renderer 输出 refs 和 typed summary，不输出 stdout/CoT。handoff payload 只能证明依赖已满足，Gate proof 仍回查 canonical rows/evidence。

### 步骤 5：GREEN 与提交

```bash
cd backend && cargo nextest run -p golish-db stage_handoff --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-kit inherited_handoff --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-runtime stage_handoff --no-tests=fail --status-level fail
```

```bash
just precommit
git add -- backend/crates/golish-db/src/repo/stage_handoffs.rs backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs backend/crates/golish-agent-kit/src/task_orchestrator/prompts/mod.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs
git diff --cached --name-only
git commit -m "feat(harness): publish durable per-organization stage handoffs"
```

---

## Task 7：exact resume、startup reaper 与诊断

**文件：** `repo/tasks.rs`、`golish/src/stage_run/mod.rs`、`scripts/run_tree.py`。

### 步骤 1：写 RED

覆盖：完整 V2 state 可恢复；dangling worker/chain、missing snapshot、wrong scope hash 被拒绝；legacy 仅在 fallback mode 恢复。

```rust
#[test]
fn startup_reaper_accepts_only_complete_owned_v2_state() {
    let complete = RuntimeResumeState::fixture_complete_v2();
    assert_eq!(classify_runtime_resume(&complete, RuntimeMemoryReadMode::V2Only), ResumeDecision::ResumeExact);
    let dangling = RuntimeResumeState { chain_owner_worker_id: None, ..complete.clone() };
    assert!(matches!(classify_runtime_resume(&dangling, RuntimeMemoryReadMode::V2Only), ResumeDecision::Reject(RuntimeResumeError::DanglingChain)));
    let missing_scope = RuntimeResumeState { scope_snapshot: None, ..complete };
    assert!(matches!(classify_runtime_resume(&missing_scope, RuntimeMemoryReadMode::V2Only), ResumeDecision::Reject(RuntimeResumeError::MissingScopeSnapshot)));
}

#[test]
fn legacy_resume_is_available_only_in_explicit_fallback_mode() {
    let legacy = RuntimeResumeState::fixture_legacy_blob_only();
    assert_eq!(classify_runtime_resume(&legacy, RuntimeMemoryReadMode::PreferV2WithLegacyFallback), ResumeDecision::ResumeLegacy);
    assert!(matches!(classify_runtime_resume(&legacy, RuntimeMemoryReadMode::V2Only), ResumeDecision::Reject(RuntimeResumeError::MissingV2State)));
}
```

### 步骤 2：更新 reaper/read model

query 必须 join operation scope、current stage unit、live worker 与 exact message chain owner；不再用 `stage_run_workers` JSON 是否非空判断 V2 可恢复性。

### 步骤 3：扩展 run_tree

`--db` 输出：

```text
scope_snapshot: id/hash/unit_count
stage_unit: id/org/stage_execution_id/stage/generation/status
worker_run: id/work_item/specialist/status/lease_owner/lease_expiry/attempt_epoch/chain/checkpoint_version
handoff: from_stage/stage_execution_id/unit_gate_decision_hash/evidence_count
legacy_fallback: true|false
```

在 `scripts/tests/test_run_tree_runtime_memory.py` 用固定 DB-row fixture 调 renderer，逐字段断言上述输出、cross-org row rejection 和 `legacy_fallback` 标记；不能只做 `py_compile`。

### 步骤 4：GREEN 与提交

```bash
cd backend && cargo nextest run -p golish-db startup_reaper --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish stage_run --no-tests=fail --status-level fail
python3 -m py_compile scripts/run_tree.py
python3 -m unittest scripts.tests.test_run_tree_runtime_memory
```

```bash
just precommit
git add -- backend/crates/golish-db/src/repo/tasks.rs backend/crates/golish/src/stage_run/mod.rs scripts/run_tree.py scripts/tests/test_run_tree_runtime_memory.py
git diff --cached --name-only
git commit -m "feat(runtime): resume stage workers from durable runtime memory"
```

---

## Task 8：迁移四个现有 specialist stage 并完成模块文档

**文件：** stage specs/methodology、runtime tests、模块卡。

### 步骤 1：参数化 stage contract 测试

对 `target_intel`、`external_attack_surface`、`enumeration`、`vuln_triage` 逐一断言：

```rust
#[test_case(StageKind::TargetIntel)]
#[test_case(StageKind::ExternalAttackSurface)]
#[test_case(StageKind::Enumeration)]
#[test_case(StageKind::VulnTriage)]
fn specialist_stage_uses_v2_runtime_unit_contract(stage: StageKind) {
    let contract = runtime_contract_for(stage).expect("specialist runtime contract");
    assert_eq!(contract.unit_identity, RuntimeUnitIdentity::StageExecutionOrganization);
    assert_eq!(contract.scope_source, RuntimeScopeSource::FrozenOperationSnapshot);
    assert!(contract.requires_worker_lease);
    assert!(contract.publishes_handoff_only_after_final_seal);
    assert!(!contract.writes_legacy_agent_run_in_v2_only);
}
```

### 步骤 2：逐 stage 切 V2Only 测试配置

每个 stage 证明：snapshot fanout、independent worker、PASS handoff、resume skip passed org。任一 stage 未通过，不把生产默认切到 V2Only。

### 步骤 3：更新模块卡

同步：

- `docs/modules/backend/golish-db{.md,/repo.md}`
- `docs/modules/backend/golish-agent-kit/{db_traits.md,task_orchestrator.md}`
- `docs/modules/backend/golish-agent-runtime/agentic_loop.md`
- `docs/modules/backend/golish-agent-app/ai.md`
- `docs/modules/backend/golish-sub-agents/executor.md`
- `docs/modules/backend/golish-agent-bridge/agent_bridge.md`
- `docs/modules/backend/golish/stage_run.md`
- `docs/modules/INDEX.md`

### 步骤 4：包级验证

```bash
cd backend && cargo nextest run -p golish-db -E 'test(operation_scope) | test(stage_run_unit) | test(stage_worker) | test(stage_handoff) | test(startup_reaper)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-app runtime_memory --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-kit task_orchestrator --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-runtime stage_run --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-sub-agents chain --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-bridge bridge_executor --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish stage_run --no-tests=fail --status-level fail
cd backend && cargo clippy -p golish-db -p golish-agent-kit -p golish-agent-app -p golish-agent-runtime -p golish-sub-agents -p golish-agent-bridge -p golish --all-targets -- -D warnings
cd backend && cargo fmt --all -- --check
just precommit
```

预期：全部 exit 0；Clippy 零 warning；所有新增测试 passed。

### 步骤 5：提交

```bash
just precommit
git add -- resources/harness/stages/target_intel/spec.json resources/harness/stages/target_intel/methodology.md resources/harness/stages/external_attack_surface/spec.json resources/harness/stages/external_attack_surface/methodology.md resources/harness/stages/enumeration/spec.json resources/harness/stages/enumeration/methodology.md resources/harness/stages/vuln_triage/spec.json resources/harness/stages/vuln_triage/methodology.md docs/modules/backend/golish-db.md docs/modules/backend/golish-db/repo.md docs/modules/backend/golish-agent-kit/db_traits.md docs/modules/backend/golish-agent-kit/task_orchestrator.md docs/modules/backend/golish-agent-runtime/agentic_loop.md docs/modules/backend/golish-agent-app/ai.md docs/modules/backend/golish-sub-agents/executor.md docs/modules/backend/golish-agent-bridge/agent_bridge.md docs/modules/backend/golish/stage_run.md docs/modules/INDEX.md agent-progress.md feature_list.json
git diff --cached --name-only
git commit -m "docs(runtime): complete runtime memory foundation rollout"
```

---

## 本计划完成后仍不做的事

- 不实现 Candidate approval/Attempt/FactDelta；由 P2 完成。
- 不创建长期 Assertion/vector/KG；由 P3-P5 完成。
- 不删除 legacy JSON 或 `org_stage_completions`；只停止 V2 主写。
- 不自动运行真实 LLM、扫描或 exploit；live smoke 另获授权。
