# Golish 运行期记忆底座修正版实现计划

> **Rollout 补充说明（2026-07-13）：** 本计划的 foundation schema、runtime state machine 与恢复链路仍然有效；仅推进 deployment default / cutover 的 rollout 步骤由 `docs/superpowers/plans/2026-07-13-runtime-memory-shadow-attestation.md` 的 retained whole-record cohort 方案取代，不作废其余基础实现。

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 为每个 operation 冻结不可漂移的组织范围，并把 stage execution、per-org Gate unit、worker lease/checkpoint、可信 deliverable submission 与 PASS handoff 从共享 JSON 迁移到有约束、可恢复、可审计的 PostgreSQL 记录。

**架构：** `runtime_memory_rollout` 保存全局、只能前进的 rollout 状态；`operation_state.runtime_memory_contract` 在 operation 创建事务里冻结该状态，运行中不可改变。`stage_runs.id` 是可信 `stage_execution_id`；Scoping 在 freeze 前只绑定 execution，freeze 事务创建 scope decision、snapshot、root unit，并把可信 submission 绑定到该 unit。后续 specialist stage 从 snapshot seed `stage_run_units`，以带 fencing token 的 `stage_worker_runs` 执行；Gate final seal 在同一事务写 unit PASS、bounded `stage_handoff` 和兼容投影。

**技术栈：** Rust 2021、sqlx/PostgreSQL、Tauri 2、React 19、TypeScript 6、ts-rs、cargo-nextest、Vitest、Python unittest。

**设计来源：** `docs/design/2026-07-12-runtime-memory-candidate-pipeline-v2.md`。本计划取代原 P1 执行顺序，但不修改或覆盖 `docs/superpowers/plans/2026-07-12-runtime-memory-foundation.md`。

**已满足前置：** 用户已授权 additive migration、`golish-db`、IPC 与实现；checkpoint `ab7b0c4a` 是本计划的干净基线。每个 Task 开始前仍须重新检查当前树，不能假定基线持续干净。

> **2026-07-12 实施审计修正（优先于下文旧 SQL sketch）：** Task 1 的 hostile RED 扩展到
> legacy duplicate/unknown status、nullable composite FK、cross-worker/submission/handoff、
> scope tree/seal、fence TOCTOU 与 payload immutability。最终 foundation migration 采用
> forward-only rollout trigger、fail-closed checksum policy、operation/project/decision composite
> ownership、`snapshot → units → sealed_at` 一次性 freeze、late-unit/worker fence row lock、
> trusted tool/submission/handoff composite identity。`stage_runs_status_check` 在 foundation 保持
> `NOT VALID`，one-active index 延后到 `00002` cutover，在确定性检查无 duplicate active row 后
> validate/create。下文 SQL sketch 与这些约束冲突处均以实际 migration +
> `runtime_memory_migrations.rs` 为准，禁止复制旧的 CASCADE、MATCH SIMPLE nullable bypass 或
> foundation 立即 VALIDATE/index 片段。Scoping pre-freeze submission 的 final bind 必须在同一
> transaction **先回填同一 tool_call 的 unit/org，再回填 submission**；两行之后均不可再改。

---

## 1. 不可变执行契约

### 1.1 Rollout 不是独立 read/write 开关

唯一合法合同如下；不存在自由组合的 3×3 matrix：

| rank | `RuntimeMemoryContract` | 权威读 | V2 写 | legacy checkpoint JSON 写 | legacy fallback |
|---:|---|---|---:|---:|---:|
| 0 | `LegacyV1` | legacy | 否 | 是 | 否 |
| 1 | `DualWriteLegacyRead` | legacy | 是 | 是，且与 V2 同事务 | 否 |
| 2 | `DualWriteV2Preferred` | 完整 V2 record | 是 | 是，且与 V2 同事务 | 仅 V2 record 整体不存在/不可解码时整条 fallback |
| 3 | `V2Only` | V2 | 是 | 否 | 否 |

- `runtime_memory_rollout` 只允许 `rank + 1`；拒绝 downgrade、skip 和重复 CAS。
- operation 创建事务读取 rollout row，并把合同写入 `operation_state.runtime_memory_contract`。
- operation 的合同由 DB trigger 保证不可修改；部署推进只影响之后创建的 operation。
- `org_stage_completions` 是兼容 read-model，不属于 legacy checkpoint JSON；P1 在 final seal 中继续原子写它。
- `DualWriteV2Preferred` 每次选择完整 V2 或完整 legacy record，禁止字段拼接。

Rust 单一事实源：

```rust
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RuntimeMemoryContract {
    #[default]
    LegacyV1,
    DualWriteLegacyRead,
    DualWriteV2Preferred,
    V2Only,
}

impl RuntimeMemoryContract {
    pub const fn can_transition_to(self, next: Self) -> bool {
        next as u8 == self as u8 || next as u8 == self as u8 + 1
    }
}
```

`backend/crates/golish-agent-kit/src/runtime_memory.rs` 已在当前工作树开工；实现者复用该文件中的 `policy()`、`RuntimeMemoryReadStrategy` 与 `RuntimeMemoryWriteStrategy`，不再创建第二套 enum。

### 1.2 可信 identity 链

| 名称 | 来源 | 模型可否提供 |
|---|---|---:|
| `project_scope_id` | trusted workspace registration | 否 |
| `operation_id` | `tasks.id` 与 `operation_state.operation_id` 的原子创建事务 | 否 |
| `stage_execution_id` | `stage_runs.id` | 否 |
| `scope_decision_id` | server 从 exact execution 的 HITL lifecycle 构造 | 否 |
| `scope_snapshot_id` | freeze transaction | 否 |
| `stage_run_unit_id` | `(stage_execution_id, organization_id)` seed | 否 |
| `worker_run_id` | `(unit, work_item_kind, work_item_key, generation)` seed | 否 |
| `tool_call_record_id` | awaited `tool_calls INSERT ... RETURNING id` | 否 |
| `deliverable_submission_id` | submit tool 的 server-side insert | 否 |
| `lease_token/attempt_epoch` | worker claim transaction | 否 |

Scoping 是唯一 freeze 前没有 `stage_run_unit_id` 的 stage。它的提交先绑定 `operation_id + stage_execution_id + tool_call_record_id`；`finalize_scoping_scope` 在一个事务中写 decision/snapshot/root unit、回填 submission unit、seal PASS 和发布 handoff。所有 post-Scoping submission 必须在执行前已有 unit identity。

### 1.3 Worker fencing 与接管

- claim transaction 同时取得 lease、递增 `attempt_epoch`、插入含 initial provider-safe chain 的 `message_chains`、绑定 chain；事务提交后才允许 provider 请求。
- heartbeat 周期 10 秒、lease TTL 30 秒；任何 heartbeat CAS 失败立即设置 `lease_lost`，禁止新工具、checkpoint 与 final landing。
- 每次工具 dispatch 前以 `worker_run_id + stage_run_unit_id + lease_token + attempt_epoch` 校验 lease，并把 `active_tool_call_id` 写入 worker。
- 工具完成后按同一 fencing tuple 清空 active call。过期 lease 若仍有 active call，reaper 转 `recovery_required`，绝不自动重放；这避免未知外部副作用重复执行。
- 过期 lease 且没有 active call 才能重新 claim。旧 worker 的 checkpoint、evidence landing 和 terminal write 会因 epoch/token 不匹配被拒绝。

### 1.4 Handoff catalog

P1 catalog 是封闭枚举，只覆盖四个 specialist stage 实际使用的 canonical truth：

```rust
pub enum CanonicalFactKey {
    Organization { organization_id: Uuid },
    Target { target_id: Uuid },
    TargetAsset { target_asset_id: Uuid },
    DnsRecord { organization_id: Uuid, domain: String, record_type: String, value: String },
    ApiEndpoint { api_endpoint_id: Uuid },
    DirectoryEntry { directory_entry_id: Uuid },
    JsAnalysisResult { js_analysis_result_id: Uuid },
    Fingerprint { fingerprint_id: Uuid },
    Finding { finding_id: Uuid },
    TechniqueOutcome { organization_id: Uuid, run_id: String, asset: String, technique: String },
}

pub struct CanonicalFactRef {
    pub key: CanonicalFactKey,
    pub organization_id: Uuid,
    pub observed_at: DateTime<Utc>,
    pub content_sha256: String,
    pub evidence_ids: Vec<i64>,
}
```

server builder 逐项回查 current row、operation/org ownership、stage freshness 与 evidence ledger。payload 限制为 256 refs、128 typed claims、1024 evidence ids、256 KiB canonical JSON；超限返回 typed error，不截断 identity。

### 1.5 P1 删除边界

P1 不伪造 P3 outbox 或 P7 cleanup obligation。只要 organization 出现在任一 P1 scope snapshot，`organization_delete` 就返回 `runtime_scope_history_requires_invalidation`，并且不进入 artifact cleanup。P3/P7 落地 durable invalidation/cleanup contract 后才能放开历史 snapshot org 删除。snapshot 不带 live organization FK，历史 identity 不会被 cascade。

---

## 2. 文件结构

### 新建

- `backend/crates/golish-db/migrations/20260712000001_runtime_memory_foundation.sql`：expand schema、rollout singleton、operation contract、runtime identities/tables/constraints。
- `backend/crates/golish-db/migrations/20260712000002_runtime_memory_v2_cutover.sql`：全部 package 验证后单步推进 singleton 到 `v2_only`；旧 operation 合同不变。
- `backend/crates/golish-db/src/repo/runtime_memory_rollout.rs`：monotonic rollout CAS。
- `backend/crates/golish-db/src/repo/project_scopes.rs`：stable workspace identity、显式 rename/retire CAS。
- `backend/crates/golish-db/src/repo/operation_scope_decisions.rs`：exact HITL/CLI decision lifecycle。
- `backend/crates/golish-db/src/repo/operation_org_scope.rs`：immutable snapshot freeze/read/hash。
- `backend/crates/golish-db/src/repo/stage_run_units.rs`：per-execution/per-org Gate unit 状态机。
- `backend/crates/golish-db/src/repo/stage_worker_runs.rs`：lease、heartbeat、active-tool fencing、checkpoint CAS。
- `backend/crates/golish-db/src/repo/stage_deliverable_submissions.rs`：可信 submission identity 与 canonical payload hash。
- `backend/crates/golish-db/src/repo/canonical_fact_refs.rs`：handoff catalog read/validation queries。
- `backend/crates/golish-db/src/repo/stage_handoffs.rs`：bounded PASS handoff read model。
- `backend/crates/golish-db/src/repo/runtime_memory_tx.rs`：所有 compound transition/dual-write/final-seal transaction。
- `backend/crates/golish-db/tests/runtime_memory_migrations.rs` 与 `backend/crates/golish-db/tests/fixtures/runtime_memory_legacy_schema.sql`：mandatory embedded PostgreSQL empty/legacy/hostile fixture。
- `backend/crates/golish-agent-kit/src/db_traits/runtime_memory.rs`：sqlx-free typed DTO、`RuntimeMemoryRepository`、`RuntimeMemoryError`。
- `backend/crates/golish-agent-kit/src/harness/handoff_catalog.rs`：closed catalog 与 bounded builder。
- `backend/crates/golish-agent-kit/src/harness/stage_runtime_contract.rs`：per-stage config types/validation；不复制 operation rollout enum。
- `backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs`：typed app bridge。
- `backend/crates/golish-agent-runtime/src/agentic_loop/worker_lease.rs`：heartbeat supervisor 与 dispatch fencing。
- `backend/crates/golish/src/stage_run/runtime_v2.rs`：CLI one-operation bootstrap/report adapter。
- `scripts/tests/test_run_tree_runtime_memory.py`：固定 runtime row renderer fixture。
- `frontend/lib/generated/{OrganizationCandidate.ts,OrganizationCandidates.ts,UnitReviewDecisionRow.ts,UnitReviewSubmission.ts}`：仅由 `just gen-types` 生成，不手写。

### 修改

- `backend/crates/golish-db/src/repo/{mod.rs,operation_state.rs,stage_runs.rs,tasks.rs,tool_calls.rs,message_chains.rs,stage_asset_waves.rs,organizations.rs}`
- `backend/crates/golish-agent-kit/src/db_traits/{mod.rs,types.rs,repo.rs,tracking.rs}`
- `backend/crates/golish-agent-kit/src/db_tracking/{mod.rs,recording.rs,types.rs}`、`backend/crates/golish-agent-kit/src/db_shim.rs`
- `backend/crates/golish-agent-kit/src/task_orchestrator/{orchestrator.rs,agent_run_checkpoint.rs,harness_resume.rs,stage_execution.rs,types.rs,prompts/mod.rs}`
- `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/{execute.rs,execute_harness_loop_tests.rs}`
- `backend/crates/golish-agent-kit/src/harness/{mod.rs,stage_spec.rs}` 与 `backend/crates/golish-agent-kit/src/harness/gate/{context_builder.rs,rule_engine.rs}`
- `backend/crates/golish-agent-app/src/ai/db_bridge/mod.rs`
- `backend/crates/golish-agent-app/src/ai/tracking_bridge/{mod.rs,records.rs,chain.rs}`
- `backend/crates/golish-agent-app/src/ai/{harness_submit_tool.rs,commands/bridge_config.rs,commands/harness_dev.rs}`
- `backend/crates/golish-core/src/agent_session.rs`
- `backend/crates/golish-agent-bridge/src/agent_bridge/{mod.rs,backends.rs,config.rs,prepare.rs,constructors/mod.rs}`
- `backend/crates/golish-agent-bridge/src/bridge_executor/trait_impl.rs`
- `backend/crates/golish-agent-runtime/src/agentic_loop/{context.rs,single_tool_call.rs}`
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/{stage_run_call.rs,sub_agent_call.rs}`
- `backend/crates/golish-sub-agents/src/executor_types.rs` 与 `backend/crates/golish-sub-agents/src/executor/{chain_persist.rs,inner.rs}`
- `backend/crates/golish-pentest-app/src/pentest_bridge/manage_organizations.rs`
- `backend/crates/golish-recon-app/src/organizations/{types.rs,candidates.rs,mod.rs,artifact_cleanup.rs}`
- `frontend/lib/api/organizations.ts`
- `frontend/components/AIChatPanel/{ScopeReviewTable.tsx,ScopeReviewTable.test.tsx,AskHumanInline.tsx,AskHumanInline.test.tsx}`
- `backend/crates/golish/src/stage_run/{mod.rs,fleet.rs,scheduler.rs}`
- `scripts/{check_repo_ownership.py,run_tree.py}`
- `resources/harness/stages/{target_intel,external_attack_surface,enumeration,vuln_triage}/spec.json`
- 本计划列出的全部模块卡、`docs/modules/INDEX.md`、`agent-progress.md`、`feature_list.json`。

---

## Task 0：重新执行开工门禁并锁定基线

**文件：** 只读检查；验证完成后更新 `agent-progress.md` 本轮记录。

### 步骤 1：检查 slot、checkpoint 与工作树

```bash
git rev-parse --short HEAD
git status --short
jq -e '[.features[] | select(.status == "in_progress")] | length == 1' feature_list.json
jq -r '.features[] | select(.status == "in_progress") | [.id,.name] | @tsv' feature_list.json
```

预期：HEAD 至少包含 `ab7b0c4a`；唯一 `in_progress` 是 runtime-memory/candidate-pipeline V2 feature。当前共享树若只包含父任务已声明的 runtime-memory Task 1 hunk，先在 `agent-progress.md` 记录 owner 后继续；出现未声明或重叠 hunk就停止协调，禁止覆盖。

### 步骤 2：读模块卡并验证基础环境

依次读 `docs/modules/INDEX.md` 以及本计划 §2 列出的模块卡，然后运行：

```bash
./init.sh
```

预期 exit 0。把命令、exit code 与关键输出写入 `agent-progress.md`；失败时保持 feature `in_progress` 并记录 blocker。

### 步骤 3：记录已知 ownership guard 基线

`scripts/check_repo_ownership.py` 在 `ab7b0c4a` 已有旧 violation，P1 不顺手清理全部历史债务。先保存数量，后续 Task 只允许数量不增加，并对本计划新增 repo 做精确注册断言：

```bash
python3 scripts/check_repo_ownership.py 2>&1 | tee /tmp/runtime-memory-ownership-baseline.txt; test ${PIPESTATUS[0]} -eq 1
rg -c '^  - ' /tmp/runtime-memory-ownership-baseline.txt
```

预期：第一条确认当前 guard 的已知非零 baseline；第二条得到固定基线数量。不得把这个历史 failure 记成 P1 新代码通过。

### 步骤 4：提交边界

Task 0 只提交 progress/feature 的会话元数据；用户未要求自动 commit 时只记录 checkpoint：

```bash
git diff --check
git diff --name-only
```

预期：没有代码或 migration 改动。

---

## Task 1：写 additive schema、约束与 migration tests

**文件：** 创建 foundation migration、十个 repo module 的 schema-facing row types/SQL constants、embedded tests/fixture；修改 `repo/mod.rs` 与 `scripts/check_repo_ownership.py`。本 Task 不创建 cutover migration。

### 步骤 1：先写 migration RED

在 `backend/crates/golish-db/tests/runtime_memory_migrations.rs` 创建三个 serial tests：

```rust
#[tokio::test]
#[serial_test::serial]
async fn foundation_migrates_empty_and_legacy_schema() {
    for fixture in [FixtureKind::Empty, FixtureKind::LegacyCheckpoint] {
        let mut pg = RuntimeMemoryPg::start(fixture).await.expect("embedded postgres");
        pg.apply_foundation().await.expect("foundation migration");
        for table in REQUIRED_RUNTIME_TABLES {
            assert!(pg.table_exists(table).await, "missing {table}");
        }
        assert_eq!(pg.rollout_contract().await, "legacy_v1");
        assert_eq!(pg.legacy_blob().await, pg.legacy_blob_before());
        pg.stop().await;
    }
}

#[tokio::test]
#[serial_test::serial]
async fn operation_contract_is_db_immutable_and_rollout_is_single_step() {
    let mut pg = RuntimeMemoryPg::start(FixtureKind::LegacyCheckpoint).await.unwrap();
    pg.apply_foundation().await.unwrap();
    assert!(pg.update_operation_contract("v2_only").await.is_err());
    assert!(pg.advance_rollout("legacy_v1", "v2_only").await.is_err());
    pg.advance_rollout("legacy_v1", "dual_write_legacy_read").await.unwrap();
    assert_eq!(pg.rollout_contract().await, "dual_write_legacy_read");
    pg.stop().await;
}

#[tokio::test]
#[serial_test::serial]
async fn hostile_cross_operation_and_cross_org_rows_are_rejected() {
    let mut pg = RuntimeMemoryPg::start(FixtureKind::LegacyCheckpoint).await.unwrap();
    pg.apply_foundation().await.unwrap();
    assert!(pg.insert_cross_operation_unit().await.is_err());
    assert!(pg.insert_cross_snapshot_handoff().await.is_err());
    assert!(pg.bind_one_chain_to_two_workers().await.is_err());
    pg.stop().await;
}
```

`RuntimeMemoryPg::start` 使用 `golish_db::embedded::EmbeddedPg`、temp `pg_data_dir`、`DbConfig::default().pg_bin_cache_dir` 和本机随机端口；不读 `GOLISH_TEST_DATABASE_URL`，也没有 skip/early return。

运行：

```bash
cd backend && cargo nextest run -p golish-db --test runtime_memory_migrations --no-tests=fail --status-level fail
```

预期 RED：缺少 test target、migration 和 runtime tables。

### 步骤 2：创建 foundation migration

`20260712000001_runtime_memory_foundation.sql` 必须完整包含下面的表/列/约束；实际 SQL 保持名称逐字一致：

```sql
CREATE TABLE runtime_memory_rollout (
    singleton_id SMALLINT PRIMARY KEY CHECK (singleton_id = 1),
    contract TEXT NOT NULL CHECK (contract IN (
        'legacy_v1','dual_write_legacy_read','dual_write_v2_preferred','v2_only'
    )),
    contract_rank SMALLINT NOT NULL CHECK (contract_rank BETWEEN 0 AND 3),
    row_version BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (contract_rank = CASE contract
        WHEN 'legacy_v1' THEN 0 WHEN 'dual_write_legacy_read' THEN 1
        WHEN 'dual_write_v2_preferred' THEN 2 WHEN 'v2_only' THEN 3 END)
);
INSERT INTO runtime_memory_rollout(singleton_id, contract, contract_rank)
VALUES (1, 'legacy_v1', 0) ON CONFLICT (singleton_id) DO NOTHING;

ALTER TABLE operation_state ADD COLUMN runtime_memory_contract TEXT NOT NULL DEFAULT 'legacy_v1';
ALTER TABLE operation_state ADD CONSTRAINT operation_state_runtime_memory_contract_check
CHECK (runtime_memory_contract IN (
    'legacy_v1','dual_write_legacy_read','dual_write_v2_preferred','v2_only'
));

CREATE FUNCTION reject_operation_runtime_contract_change() RETURNS trigger AS $$
BEGIN
    IF NEW.runtime_memory_contract IS DISTINCT FROM OLD.runtime_memory_contract THEN
        RAISE EXCEPTION 'operation runtime memory contract is immutable';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
CREATE TRIGGER operation_runtime_contract_immutable
BEFORE UPDATE OF runtime_memory_contract ON operation_state
FOR EACH ROW EXECUTE FUNCTION reject_operation_runtime_contract_change();

ALTER TABLE stage_runs ADD CONSTRAINT stage_runs_id_operation_unique UNIQUE(id, operation_id);
ALTER TABLE stage_runs ADD CONSTRAINT stage_runs_status_check
CHECK (status IN ('started','completed','failed','paused_needs_user')) NOT VALID;
ALTER TABLE stage_runs VALIDATE CONSTRAINT stage_runs_status_check;
CREATE UNIQUE INDEX stage_runs_one_active_per_operation
ON stage_runs(operation_id) WHERE status = 'started';

CREATE TABLE project_scopes (
    project_scope_id UUID PRIMARY KEY,
    canonical_project_path TEXT NOT NULL,
    path_sha256 TEXT NOT NULL,
    row_version BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    retired_at TIMESTAMPTZ
);
CREATE UNIQUE INDEX project_scopes_one_active_path
ON project_scopes(canonical_project_path) WHERE retired_at IS NULL;

ALTER TABLE operation_state
ADD COLUMN project_scope_id UUID REFERENCES project_scopes(project_scope_id) ON DELETE RESTRICT;

CREATE TABLE operation_scope_decisions (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE CASCADE,
    stage_execution_id UUID NOT NULL,
    root_organization_id UUID NOT NULL,
    mode TEXT NOT NULL CHECK (mode IN ('root_only','included','reuse_reconfirmed','cli_flags')),
    choice_tool_call_id UUID,
    proposal_tool_call_id UUID,
    review_tool_call_id UUID,
    decision_rows JSONB NOT NULL,
    decision_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(operation_id, stage_execution_id),
    FOREIGN KEY(stage_execution_id, operation_id) REFERENCES stage_runs(id, operation_id)
);

CREATE TABLE operation_org_scope_snapshots (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL UNIQUE REFERENCES operation_state(operation_id) ON DELETE CASCADE,
    project_scope_id UUID NOT NULL REFERENCES project_scopes(project_scope_id) ON DELETE RESTRICT,
    scope_decision_id UUID NOT NULL UNIQUE REFERENCES operation_scope_decisions(id),
    project_path_at_freeze TEXT NOT NULL,
    root_organization_id UUID NOT NULL,
    mode TEXT NOT NULL,
    scope_hash TEXT NOT NULL,
    schema_version INTEGER NOT NULL DEFAULT 1,
    frozen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(id, operation_id)
);

CREATE TABLE operation_org_scope_units (
    snapshot_id UUID NOT NULL REFERENCES operation_org_scope_snapshots(id) ON DELETE CASCADE,
    organization_id UUID NOT NULL,
    parent_organization_id UUID,
    organization_name_at_freeze TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('root','subsidiary')),
    depth INTEGER NOT NULL CHECK (depth >= 0),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    ownership_percent NUMERIC,
    decision_row_id TEXT NOT NULL,
    approval_source JSONB NOT NULL,
    PRIMARY KEY(snapshot_id, organization_id),
    UNIQUE(snapshot_id, ordinal)
);

CREATE TABLE stage_run_units (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id) ON DELETE CASCADE,
    stage_execution_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    stage_kind TEXT NOT NULL,
    generation INTEGER NOT NULL CHECK (generation >= 0),
    specialist TEXT,
    status TEXT NOT NULL CHECK (status IN (
        'queued','running','gate_blocked','passed','exhausted','superseded'
    )),
    gate_attempt INTEGER NOT NULL DEFAULT 0 CHECK (gate_attempt >= 0),
    pass_watermark JSONB NOT NULL DEFAULT '{}'::jsonb,
    row_version BIGINT NOT NULL DEFAULT 0,
    started_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    terminal_at TIMESTAMPTZ,
    UNIQUE(stage_execution_id, organization_id),
    UNIQUE(id, operation_id, stage_execution_id, organization_id),
    FOREIGN KEY(stage_execution_id, operation_id) REFERENCES stage_runs(id, operation_id),
    FOREIGN KEY(scope_snapshot_id, operation_id)
        REFERENCES operation_org_scope_snapshots(id, operation_id),
    FOREIGN KEY(scope_snapshot_id, organization_id)
        REFERENCES operation_org_scope_units(snapshot_id, organization_id)
);

ALTER TABLE tool_calls
    ADD COLUMN operation_id UUID,
    ADD COLUMN stage_execution_id UUID,
    ADD COLUMN stage_run_unit_id UUID,
    ADD COLUMN worker_run_id UUID,
    ADD COLUMN organization_id UUID,
    ADD COLUMN attempt_epoch BIGINT,
    ADD COLUMN lease_token UUID;

CREATE TABLE stage_worker_runs (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    worker_generation INTEGER NOT NULL CHECK (worker_generation >= 0),
    specialist TEXT NOT NULL,
    work_item_kind TEXT NOT NULL,
    work_item_key TEXT NOT NULL,
    agent_path TEXT NOT NULL,
    parent_request_id TEXT,
    message_chain_id UUID REFERENCES message_chains(id) ON DELETE RESTRICT,
    status TEXT NOT NULL CHECK (status IN (
        'queued','running','waiting_background','gate_blocked','passed','failed',
        'exhausted','superseded','recovery_required'
    )),
    gate_attempt INTEGER NOT NULL DEFAULT 0,
    checkpoint JSONB NOT NULL DEFAULT '{}'::jsonb,
    checkpoint_version BIGINT NOT NULL DEFAULT 0,
    lease_token UUID,
    lease_owner TEXT,
    lease_acquired_at TIMESTAMPTZ,
    lease_expires_at TIMESTAMPTZ,
    heartbeat_at TIMESTAMPTZ,
    attempt_epoch BIGINT NOT NULL DEFAULT 0,
    active_tool_call_id UUID REFERENCES tool_calls(id) ON DELETE RESTRICT,
    active_tool_started_at TIMESTAMPTZ,
    evidence_watermark BIGINT,
    started_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    terminal_at TIMESTAMPTZ,
    UNIQUE(stage_run_unit_id, work_item_kind, work_item_key, worker_generation),
    FOREIGN KEY(stage_run_unit_id, operation_id, stage_execution_id, organization_id)
        REFERENCES stage_run_units(id, operation_id, stage_execution_id, organization_id),
    CHECK ((lease_token IS NULL AND lease_owner IS NULL AND lease_expires_at IS NULL)
        OR (lease_token IS NOT NULL AND lease_owner IS NOT NULL AND lease_expires_at IS NOT NULL)),
    CHECK ((active_tool_call_id IS NULL AND active_tool_started_at IS NULL)
        OR (active_tool_call_id IS NOT NULL AND active_tool_started_at IS NOT NULL))
);
CREATE UNIQUE INDEX stage_worker_runs_chain_owner
ON stage_worker_runs(message_chain_id) WHERE message_chain_id IS NOT NULL;

ALTER TABLE tool_calls ADD CONSTRAINT tool_calls_stage_execution_fk
FOREIGN KEY(stage_execution_id, operation_id) REFERENCES stage_runs(id, operation_id);
ALTER TABLE tool_calls ADD CONSTRAINT tool_calls_stage_unit_fk
FOREIGN KEY(stage_run_unit_id, operation_id, stage_execution_id, organization_id)
REFERENCES stage_run_units(id, operation_id, stage_execution_id, organization_id);
ALTER TABLE tool_calls ADD CONSTRAINT tool_calls_worker_fence_fk
FOREIGN KEY(worker_run_id) REFERENCES stage_worker_runs(id);

CREATE TABLE stage_deliverable_submissions (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL,
    stage_execution_id UUID NOT NULL,
    stage_run_unit_id UUID,
    worker_run_id UUID,
    organization_id UUID,
    tool_call_record_id UUID NOT NULL UNIQUE REFERENCES tool_calls(id) ON DELETE RESTRICT,
    tool_request_id TEXT NOT NULL,
    stage_kind TEXT NOT NULL,
    attempt_epoch BIGINT,
    lease_token UUID,
    payload JSONB NOT NULL,
    payload_sha256 TEXT NOT NULL,
    submitted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(id, operation_id, stage_execution_id),
    FOREIGN KEY(stage_execution_id, operation_id) REFERENCES stage_runs(id, operation_id),
    FOREIGN KEY(stage_run_unit_id, operation_id, stage_execution_id, organization_id)
        REFERENCES stage_run_units(id, operation_id, stage_execution_id, organization_id),
    FOREIGN KEY(worker_run_id) REFERENCES stage_worker_runs(id),
    CHECK ((stage_kind = 'scoping' AND worker_run_id IS NULL)
        OR stage_run_unit_id IS NOT NULL)
);

CREATE TABLE stage_handoffs (
    id UUID PRIMARY KEY,
    operation_id UUID NOT NULL,
    organization_id UUID NOT NULL,
    scope_snapshot_id UUID NOT NULL,
    from_stage_kind TEXT NOT NULL,
    stage_execution_id UUID NOT NULL,
    source_stage_run_unit_id UUID NOT NULL UNIQUE,
    deliverable_submission_id UUID NOT NULL UNIQUE,
    scope_hash TEXT NOT NULL,
    payload JSONB NOT NULL,
    payload_sha256 TEXT NOT NULL,
    evidence_ids BIGINT[] NOT NULL DEFAULT '{}',
    coverage_watermark JSONB NOT NULL DEFAULT '{}'::jsonb,
    unit_gate_decision_hash TEXT NOT NULL,
    aggregate_pass_token_hash TEXT,
    gate_passed_at TIMESTAMPTZ NOT NULL,
    invalidated_at TIMESTAMPTZ,
    schema_version INTEGER NOT NULL DEFAULT 1,
    UNIQUE(stage_execution_id, organization_id),
    FOREIGN KEY(scope_snapshot_id, operation_id)
        REFERENCES operation_org_scope_snapshots(id, operation_id),
    FOREIGN KEY(scope_snapshot_id, organization_id)
        REFERENCES operation_org_scope_units(snapshot_id, organization_id),
    FOREIGN KEY(source_stage_run_unit_id, operation_id, stage_execution_id, organization_id)
        REFERENCES stage_run_units(id, operation_id, stage_execution_id, organization_id),
    FOREIGN KEY(deliverable_submission_id, operation_id, stage_execution_id)
        REFERENCES stage_deliverable_submissions(id, operation_id, stage_execution_id)
);
```

注意：`operation_org_scope_units.organization_id` 没有 live `organizations` FK，这是保留冻结 identity 的刻意设计；所有写入仍由 repo 在 freeze 前验证 live org 的 project ownership。

### 步骤 3：注册 repo ownership

每个新 repo 文件先落可编译的 DB row type、status enum、table/constraint 常量以及对应 SQL contract test；mutation 方法在其所属后续 Task 以 TDD 加入。随后在 `repo/mod.rs` 注册十个 module，并在 `scripts/check_repo_ownership.py` 的 `REPO_OWNER` 把它们登记为 `agent`：

```python
"runtime_memory_rollout": "agent",
"project_scopes": "agent",
"operation_scope_decisions": "agent",
"operation_org_scope": "agent",
"stage_run_units": "agent",
"stage_worker_runs": "agent",
"stage_deliverable_submissions": "agent",
"canonical_fact_refs": "agent",
"stage_handoffs": "agent",
"runtime_memory_tx": "agent",
```

不把这些 repo 塞进 `SHARED_REPOS`，也不新增 cross-service allowlist。CLI 和 recon deletion 通过 kit trait/app port 访问，避免 command-layer 直接跨 owner。

精确断言：

```bash
python3 -c 'import scripts.check_repo_ownership as g; expected={"runtime_memory_rollout","project_scopes","operation_scope_decisions","operation_org_scope","stage_run_units","stage_worker_runs","stage_deliverable_submissions","canonical_fact_refs","stage_handoffs","runtime_memory_tx"}; missing=expected-set(g.REPO_OWNER); assert not missing, missing'
python3 scripts/check_repo_ownership.py 2>&1 | tee /tmp/runtime-memory-ownership-after-task1.txt; test ${PIPESTATUS[0]} -eq 1
test "$(rg -c '^  - ' /tmp/runtime-memory-ownership-after-task1.txt)" -le "$(rg -c '^  - ' /tmp/runtime-memory-ownership-baseline.txt)"
```

预期：新增 repo 全部有 owner；历史 violation 数不增加。

### 步骤 4：GREEN

```bash
cd backend && cargo nextest run -p golish-db --test runtime_memory_migrations --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-db -E 'test(runtime_memory) | test(stage_run)' --no-tests=fail --status-level fail
git diff --check
```

预期：三个 embedded tests passed；legacy fixture 的 JSON 未改变；hostile FK fixtures 被拒绝。

### 步骤 5：提交

```bash
git add -- backend/crates/golish-db/migrations/20260712000001_runtime_memory_foundation.sql backend/crates/golish-db/src/repo/mod.rs backend/crates/golish-db/src/repo/runtime_memory_rollout.rs backend/crates/golish-db/src/repo/project_scopes.rs backend/crates/golish-db/src/repo/operation_scope_decisions.rs backend/crates/golish-db/src/repo/operation_org_scope.rs backend/crates/golish-db/src/repo/stage_run_units.rs backend/crates/golish-db/src/repo/stage_worker_runs.rs backend/crates/golish-db/src/repo/stage_deliverable_submissions.rs backend/crates/golish-db/src/repo/canonical_fact_refs.rs backend/crates/golish-db/src/repo/stage_handoffs.rs backend/crates/golish-db/src/repo/runtime_memory_tx.rs backend/crates/golish-db/tests/runtime_memory_migrations.rs backend/crates/golish-db/tests/fixtures/runtime_memory_legacy_schema.sql scripts/check_repo_ownership.py
git diff --cached --name-only
git commit -m "feat(runtime): add constrained runtime memory schema"
```

预期：暂存区不含 `20260712000002_runtime_memory_v2_cutover.sql`，也不含 runtime 执行代码。

---

## Task 2：持久化 monotonic rollout，并原子创建 Task + operation

**文件：** `backend/crates/golish-agent-kit/src/runtime_memory.rs`、`backend/crates/golish-agent-kit/src/db_traits/{mod.rs,runtime_memory.rs,types.rs,repo.rs}`、`backend/crates/golish-agent-kit/src/db_shim.rs`、`backend/crates/golish-db/src/repo/{runtime_memory_rollout.rs,project_scopes.rs,runtime_memory_tx.rs,operation_state.rs,tasks.rs}`、`backend/crates/golish-agent-app/src/ai/db_bridge/{mod.rs,runtime_memory.rs,orchestration.rs}`、`backend/crates/golish-agent-app/src/ai/commands/core/chat.rs`、`backend/crates/golish-agent-kit/src/task_orchestrator/{orchestrator.rs,types.rs}`、`backend/crates/golish/src/stage_run/mod.rs`。

### 步骤 1：写 contract 与 atomic-create RED

在 `runtime_memory.rs` 保留当前四态命名，增加 property-style cases：

```rust
#[test]
fn every_contract_has_one_safe_policy_and_only_adjacent_progression() {
    for contract in RuntimeMemoryContract::ALL {
        let policy = contract.policy();
        assert!(!policy.may_merge_fields_from_two_sources);
        for next in RuntimeMemoryContract::ALL {
            assert_eq!(
                contract.can_transition_to(next),
                next == contract || next as u8 == contract as u8 + 1,
            );
        }
    }
}
```

在 `runtime_memory_tx.rs` 先写 embedded repository tests：

```rust
#[tokio::test]
async fn create_runtime_operation_freezes_persisted_rollout_and_project_scope() {
    let fixture = RuntimeMemoryFixture::new().await;
    fixture.advance_rollout(RuntimeMemoryContract::DualWriteLegacyRead).await.unwrap();
    let created = fixture.create_operation("/tmp/ws-a").await.unwrap();
    assert_eq!(created.operation.runtime_memory_contract, RuntimeMemoryContract::DualWriteLegacyRead);
    assert_eq!(created.operation.project_scope_id, created.project_scope.project_scope_id);
    fixture.advance_rollout(RuntimeMemoryContract::DualWriteV2Preferred).await.unwrap();
    assert_eq!(fixture.load_operation(created.task.id).await.runtime_memory_contract,
        RuntimeMemoryContract::DualWriteLegacyRead);
}

#[tokio::test]
async fn task_and_operation_insert_roll_back_together() {
    let fixture = RuntimeMemoryFixture::new().await;
    fixture.fail_after_task_insert();
    let operation_id = Uuid::new_v4();
    assert!(fixture.create_operation_with_id(operation_id, "/tmp/ws-a").await.is_err());
    assert!(fixture.task(operation_id).await.is_none());
    assert!(fixture.operation(operation_id).await.is_none());
}

#[tokio::test]
async fn rollout_rejects_skip_downgrade_and_stale_row_version() {
    let fixture = RuntimeMemoryFixture::new().await;
    assert!(matches!(
        fixture.advance_from(RuntimeMemoryContract::LegacyV1, RuntimeMemoryContract::V2Only, 0).await,
        Err(RuntimeMemoryError::InvalidContractTransition { .. })
    ));
    fixture.advance_from(RuntimeMemoryContract::LegacyV1,
        RuntimeMemoryContract::DualWriteLegacyRead, 0).await.unwrap();
    assert!(matches!(
        fixture.advance_from(RuntimeMemoryContract::DualWriteLegacyRead,
            RuntimeMemoryContract::DualWriteV2Preferred, 0).await,
        Err(RuntimeMemoryError::StaleVersion { .. })
    ));
}
```

RED：

```bash
cd backend && cargo nextest run -p golish-agent-kit runtime_memory_contract --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-db -E 'test(create_runtime_operation) | test(rollout_rejects)' --no-tests=fail --status-level fail
```

预期：kit 当前 enum test 可部分通过；DB tests 因 repo/compound API 缺失失败。

### 步骤 2：定义 typed contract；不把 runtime 错误藏进 `anyhow`

`db_traits/runtime_memory.rs` 定义：

```rust
#[derive(Debug, thiserror::Error)]
pub enum RuntimeMemoryError {
    #[error("runtime memory repository is unavailable")]
    Unavailable,
    #[error("invalid runtime contract transition: {from} -> {to}")]
    InvalidContractTransition { from: RuntimeMemoryContract, to: RuntimeMemoryContract },
    #[error("stale runtime row version: expected {expected}")]
    StaleVersion { expected: i64 },
    #[error("runtime memory conflict: {code}")]
    Conflict { code: &'static str },
    #[error("runtime memory identity mismatch: {code}")]
    IdentityMismatch { code: &'static str },
    #[error("runtime memory row missing: {entity}")]
    Missing { entity: &'static str },
    #[error("runtime worker lease lost: worker={worker_run_id}, epoch={attempt_epoch}")]
    LeaseLost { worker_run_id: Uuid, attempt_epoch: i64 },
    #[error("runtime memory storage failure: {0}")]
    Storage(String),
}

#[derive(Debug, Clone)]
pub struct ProjectScopeRegistration {
    pub project_scope_id: Uuid,
    pub canonical_project_path: String,
    pub path_sha256: String,
    pub row_version: i64,
}

#[derive(Debug, Clone)]
pub struct CreateRuntimeOperation {
    pub operation_id: Uuid,
    pub session_id: Uuid,
    pub title: Option<String>,
    pub input: String,
    pub profile: String,
    pub entry_stage: String,
    pub project_scope: ProjectScopeRegistration,
}

#[derive(Debug, Clone)]
pub struct CreatedRuntimeOperation {
    pub task: TaskView,
    pub operation: OperationStateView,
}
```

`RuntimeMemoryRepository` 的 operation-start 方法返回 `Result<_, RuntimeMemoryError>`；test 直接 `matches!(error, RuntimeMemoryError::...)`，不对 `anyhow::Error` 做错误类型匹配：

```rust
#[async_trait::async_trait]
pub trait RuntimeMemoryRepository: Send + Sync {
    async fn project_scope_register_first_open(
        &self,
        canonical_path: &str,
        path_sha256: &str,
    ) -> Result<ProjectScopeRegistration, RuntimeMemoryError>;

    async fn project_scope_rename(
        &self,
        project_scope_id: Uuid,
        expected_old_path: &str,
        expected_row_version: i64,
        new_path: &str,
        new_path_sha256: &str,
    ) -> Result<ProjectScopeRegistration, RuntimeMemoryError>;

    async fn create_runtime_operation(
        &self,
        input: CreateRuntimeOperation,
    ) -> Result<CreatedRuntimeOperation, RuntimeMemoryError>;
}
```

`GolishDbRepoProvider` 同时实现既有 `DbRepoProvider` 与新 trait。TaskOrchestrator 新增 `runtime_repo: Arc<dyn RuntimeMemoryRepository>` 构造参数；不要给 production path 一个 non-atomic fallback implementation。

### 步骤 3：实现 project path identity

chat 与 CLI 只从 trusted workspace 构造 registration：

```rust
pub fn canonical_workspace_identity(path: &Path) -> Result<(String, String), RuntimeMemoryError> {
    use sha2::{Digest, Sha256};
    let canonical = std::fs::canonicalize(path)
        .map_err(|e| RuntimeMemoryError::Storage(format!("canonicalize workspace: {e}")))?;
    if !canonical.is_dir() {
        return Err(RuntimeMemoryError::Conflict { code: "workspace_not_directory" });
    }
    let text = canonical.to_string_lossy().into_owned();
    let sha256 = Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok((text, sha256))
}
```

- 不 lower-case，不展开一个不存在的 path，不按 basename 猜旧 project。
- 同一 active canonical path 返回原 `project_scope_id`。
- rename 必须提交 expected id/path/row-version；只给新 path 时不推断为 rename。
- operation/session/runtime authorization 使用 `project_scope_id`；path 只是 provenance。

Task 1 的 foundation migration 已增加 nullable `operation_state.project_scope_id` FK。legacy row 可以为 NULL；所有新 runtime operation 必须非 NULL，由 compound insert 保证。Task 2 不回改已提交 migration。

### 步骤 4：实现 rollout CAS 与 compound operation create

`runtime_memory_rollout::advance` 只执行：

```sql
UPDATE runtime_memory_rollout
SET contract = $2,
    contract_rank = $3,
    row_version = row_version + 1,
    updated_at = NOW()
WHERE singleton_id = 1
  AND contract = $1
  AND contract_rank + 1 = $3
  AND row_version = $4
RETURNING contract, contract_rank, row_version, updated_at
```

rows affected 0 映射为 `InvalidContractTransition` 或 `StaleVersion`，通过同事务 `SELECT` 区分。

`runtime_memory_tx::create_runtime_operation`：

```rust
pub async fn create_runtime_operation(
    pool: &PgPool,
    input: &CreateRuntimeOperationRow,
) -> Result<CreatedRuntimeOperationRow, RuntimeMemoryError> {
    let mut tx = pool.begin().await.map_err(storage)?;
    let rollout = runtime_memory_rollout::get_for_share(&mut *tx).await?;
    let task = tasks::insert_with_id(&mut *tx, input.operation_id, input.session_id,
        input.title.as_deref(), &input.input).await?;
    let operation = operation_state::insert_with_executor(
        &mut *tx,
        input.operation_id,
        &input.profile,
        &input.entry_stage,
        rollout.contract.as_str(),
        input.project_scope_id,
    ).await?;
    tx.commit().await.map_err(storage)?;
    Ok(CreatedRuntimeOperationRow { task, operation })
}
```

生产 orchestrator 不再调用 `tasks::create` 后单独 `operation_state::insert`。当前工作树中临时加入的 `TaskOrchestrator::set_runtime_memory_contract` 只用于初始 RED，GREEN 时删除：合同从 persisted singleton 读取，不能由 chat/CLI/model/request 参数选择。

### 步骤 5：TaskOrchestrator 与 app/CLI wiring

构造 production orchestrator 时共享一个 concrete provider：

```rust
let provider = Arc::new(GolishDbRepoProvider::new(state.db_pool.clone()));
let db_repo: Arc<dyn DbRepoProvider> = provider.clone();
let runtime_repo: Arc<dyn RuntimeMemoryRepository> = provider;
let mut orchestrator = TaskOrchestrator::new(db_repo, runtime_repo, uuid_session_id, event_tx);
```

chat 取 `bridge.workspace().read().await`；CLI 使用 `args.resolve_workspace()` 的 canonical result。两条路径先 `project_scope_register_first_open`，再把 registration 传给 `run`/`run_stage_with_scope`。resume 不重新选择合同或 scope id，只读原 operation。

operation insert、project registration 或 rollout decode 任一失败：Task 不存在或保持 failed，不继续 LLM/provider。

### 步骤 6：GREEN

```bash
cd backend && cargo nextest run -p golish-agent-kit runtime_memory_contract --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-db -E 'test(create_runtime_operation) | test(rollout_rejects) | test(project_scope)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-app operation_runtime_contract --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-kit task_and_operation_insert_roll_back_together --no-tests=fail --status-level fail
git diff --check
```

预期：全部 passed；不存在“operation insert failed (continuing)”日志分支。

### 步骤 7：提交

```bash
git add -- backend/crates/golish-agent-kit/src/runtime_memory.rs backend/crates/golish-agent-kit/src/db_traits/mod.rs backend/crates/golish-agent-kit/src/db_traits/runtime_memory.rs backend/crates/golish-agent-kit/src/db_traits/types.rs backend/crates/golish-agent-kit/src/db_traits/repo.rs backend/crates/golish-agent-kit/src/db_shim.rs backend/crates/golish-db/src/repo/runtime_memory_rollout.rs backend/crates/golish-db/src/repo/project_scopes.rs backend/crates/golish-db/src/repo/runtime_memory_tx.rs backend/crates/golish-db/src/repo/operation_state.rs backend/crates/golish-db/src/repo/tasks.rs backend/crates/golish-agent-app/src/ai/db_bridge/mod.rs backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs backend/crates/golish-agent-app/src/ai/db_bridge/orchestration.rs backend/crates/golish-agent-app/src/ai/commands/core/chat.rs backend/crates/golish-agent-kit/src/task_orchestrator/orchestrator.rs backend/crates/golish-agent-kit/src/task_orchestrator/types.rs backend/crates/golish/src/stage_run/mod.rs
git diff --cached --name-only
git commit -m "feat(runtime): freeze persisted operation runtime contract"
```

预期：commit 包含当前已开工的 safe enum/operation column hunk，并补齐 persisted singleton、project identity 与原子创建；不包含 worker 或 handoff。

---

## Task 3：建立可信 StageExecution、tool-call 与 deliverable submission identity

**文件：** `backend/crates/golish-db/src/repo/{stage_runs.rs,tool_calls.rs,stage_deliverable_submissions.rs,runtime_memory_tx.rs}`、`backend/crates/golish-agent-kit/src/db_traits/{runtime_memory.rs,tracking.rs,types.rs}`、`backend/crates/golish-agent-kit/src/db_tracking/{mod.rs,recording.rs,types.rs}`、`backend/crates/golish-agent-kit/src/task_orchestrator/{stage_execution.rs,harness_resume.rs,agent_run_checkpoint.rs,orchestrator.rs,types.rs}`、`backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/{execute.rs,execute_harness_loop_tests.rs}`、`backend/crates/golish-core/src/agent_session.rs`、`backend/crates/golish-agent-app/src/ai/tracking_bridge/{mod.rs,records.rs}`、`backend/crates/golish-agent-app/src/ai/{harness_submit_tool.rs,db_bridge/runtime_memory.rs}`、`backend/crates/golish-agent-bridge/src/agent_bridge/{mod.rs,config.rs,prepare.rs,constructors/mod.rs}`、`backend/crates/golish-agent-bridge/src/bridge_executor/trait_impl.rs`、`backend/crates/golish-agent-runtime/src/agentic_loop/{context.rs,single_tool_call.rs}`。

### 步骤 1：写 identity RED

```rust
#[tokio::test]
async fn stage_transition_is_atomic_and_has_one_active_execution() {
    let fixture = RuntimeMemoryFixture::operation_at(StageKind::Scoping).await;
    let current = fixture.active_stage_execution().await;
    fixture.fail_after_new_stage_run_insert();
    assert!(fixture.transition_to(StageKind::TargetIntel).await.is_err());
    assert_eq!(fixture.operation_stage().await, StageKind::Scoping);
    assert_eq!(fixture.active_stage_execution().await, current);
    assert_eq!(fixture.active_stage_execution_count().await, 1);
}

#[tokio::test]
async fn submit_uses_trusted_execution_and_persisted_tool_call_identity() {
    let fixture = SubmitFixture::v2_scoping().await;
    let out = fixture.submit(json!({
        "stage_id": "scoping",
        "stage_run_id": Uuid::new_v4(),
        "claims": []
    })).await.unwrap();
    let submission = fixture.load_submission(out.deliverable_submission_id).await;
    assert_eq!(submission.stage_execution_id, fixture.stage_execution_id());
    assert_eq!(submission.tool_call_record_id, fixture.tool_call_record_id());
    assert_eq!(submission.payload["stage_run_id"], fixture.stage_execution_id().to_string());
}

#[tokio::test]
async fn v2_stage_tool_does_not_execute_when_tracking_insert_fails() {
    let fixture = ToolDispatchFixture::v2_worker();
    fixture.fail_tool_call_insert();
    assert!(matches!(fixture.dispatch("nmap").await,
        Err(RuntimeMemoryError::Storage(_))));
    assert_eq!(fixture.tool_executor_calls(), 0);
}

#[tokio::test]
async fn only_scoping_submission_may_start_without_a_unit() {
    let fixture = SubmitFixture::v2_without_unit().await;
    assert!(fixture.submit_for(StageKind::Scoping).await.is_ok());
    assert!(matches!(fixture.submit_for(StageKind::TargetIntel).await,
        Err(RuntimeMemoryError::IdentityMismatch { code: "missing_stage_run_unit" })));
}
```

RED：

```bash
cd backend && cargo nextest run -p golish-db -E 'test(stage_transition_is_atomic) | test(submit_uses_trusted)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-runtime v2_stage_tool_does_not_execute --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-app -E 'test(submit_uses_trusted) | test(only_scoping)' --no-tests=fail --status-level fail
```

### 步骤 2：让 operation creation 同时打开 initial stage execution

回到 Task 2 的 `CreateRuntimeOperation`，增加 server-preallocated `initial_stage_execution_id`；compound transaction 在插入 operation 后调用 `stage_runs::insert_with_executor`。`CreatedRuntimeOperation` 返回该 ID。

`stage_runs.rs` 改成 typed transition，不接受任意 status string：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageExecutionTerminal { Completed, Failed, PausedNeedsUser }

pub async fn mark_terminal_cas<'e, E>(
    executor: E,
    operation_id: Uuid,
    stage_execution_id: Uuid,
    terminal: StageExecutionTerminal,
) -> Result<(), RuntimeMemoryError>
where E: Executor<'e, Database = Postgres> {
    // UPDATE ... WHERE id=$1 AND operation_id=$2 AND status='started'
    // rows_affected must equal one.
}
```

`runtime_memory_tx::transition_stage_execution` 的短事务固定顺序：锁 `operation_state FOR UPDATE` → CAS close old `stage_runs` → insert new `stage_runs` → update `operation_state.current_stage/stage_started_at` → 按 frozen contract 原子写 legacy `HarnessResumeState` mirror → commit。任何一步失败全部 rollback。

`ExecutionContext` 增加：

```rust
pub stage_execution_id: Option<Uuid>,
pub stage_run_unit_id: Option<Uuid>,
pub worker_lease: Option<WorkerLeaseContext>,
```

fresh run 使用 compound create 返回值；resume 从 DB exact active row读取。禁止用 task id、tool request id 或随机 UUID 代替 stage execution。

### 步骤 3：让 tool-call tracking 返回 awaited DB UUID

定义 sqlx-free identity：

```rust
#[derive(Debug, Clone)]
pub struct RuntimeToolIdentity {
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Option<Uuid>,
    pub worker_run_id: Option<Uuid>,
    pub organization_id: Option<Uuid>,
    pub attempt_epoch: Option<i64>,
    pub lease_token: Option<Uuid>,
}

pub struct ToolCallGuard {
    pub record_id: Option<Uuid>,
    pub call_id: String,
    pub session_uuid: Uuid,
    pub started_at: Instant,
}
```

`DbTrackingBackend::record_tool_call_start` 改为：

```rust
async fn record_tool_call_start(
    &self,
    call_id: &str,
    session_id: Uuid,
    task_id: Option<Uuid>,
    subtask_id: Option<Uuid>,
    tool_name: &str,
    args: &Value,
    runtime: Option<&RuntimeToolIdentity>,
) -> anyhow::Result<Uuid>;
```

Pg implementation 执行 `INSERT ... RETURNING id`，写入 Task 1 新列；不再 `ON CONFLICT DO NOTHING` 后丢失 row id。finish 按 `id + session_id` 更新，要求 rows affected = 1。

普通非-stage chat 若 tracking 失败可以保留 best-effort telemetry；`runtime_identity.is_some()` 且 operation contract 非 `LegacyV1` 时必须在 tool executor 前返回错误。

### 步骤 4：扩展 trusted task-local tool context

`golish-core/src/agent_session.rs`：

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerLeaseContext {
    pub worker_run_id: Uuid,
    pub stage_run_unit_id: Uuid,
    pub lease_token: Uuid,
    pub attempt_epoch: i64,
}

pub struct AgentToolContext {
    pub request_id: String,
    pub tool_call_record_id: Option<Uuid>,
    pub tool_name: String,
    pub source: ToolSource,
    pub operation_id: Option<Uuid>,
    pub stage_execution_id: Option<Uuid>,
    pub stage_run_unit_id: Option<Uuid>,
    pub organization_id: Option<Uuid>,
    pub worker_lease: Option<WorkerLeaseContext>,
}
```

`single_tool_call.rs` 先 await tracking insert，再构造 context；不反过来。bridge 的 active handles 增加 execution/unit/worker identity，并在每次 subtask 结束和 top-level cleanup 时清空，避免跨 stage 污染。

### 步骤 5：持久化可信 deliverable submission

`SubmitStageDeliverableTool` 注入 `Arc<dyn RuntimeMemoryRepository>`，把 `last_deliverable: Option<String>` 升级为：

```rust
#[derive(Debug, Clone)]
pub struct CapturedStageSubmission {
    pub deliverable_submission_id: Uuid,
    pub stage_execution_id: Uuid,
    pub stage_run_unit_id: Option<Uuid>,
    pub canonical_deliverable_json: String,
}
```

execute 顺序：

1. 解析模型字段。
2. 读取 `current_agent_tool_context()`；V2 缺 operation/execution/tool-call record 立即 reject。
3. post-Scoping 缺 unit 立即 reject。
4. 覆写 `deliverable.stage_run_id = trusted.stage_execution_id`。
5. canonical JSON（对象 key 排序、UTF-8、无 whitespace）并算 SHA-256。
6. `stage_deliverable_submission_insert` 校验 tool-call row 的 operation/execution/unit/worker/epoch/token 与 context 完全相等，再 insert。
7. side-channel 保存 typed capture；tool response 返回 submission id。

```rust
Ok(json!({
    "status": "accepted",
    "deliverable_submission_id": submission.id,
    "stage_execution_id": submission.stage_execution_id,
    "stage_run_unit_id": submission.stage_run_unit_id,
}))
```

Gate close 不再信任 prose JSON 里的 `stage_run_id`；它使用 captured submission id 回读 canonical row。legacy-only direct tests可使用显式 `LegacySubmissionFixture`，production V2 不走这个 seam。

### 步骤 6：GREEN

```bash
cd backend && cargo nextest run -p golish-db -E 'test(stage_transition_is_atomic) | test(stage_deliverable_submission)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-kit -E 'test(stage_execution) | test(tool_call_guard)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-runtime -E 'test(v2_stage_tool_does_not_execute) | test(trusted_tool_context)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-app -E 'test(submit_uses_trusted) | test(only_scoping)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-bridge bridge_executor --no-tests=fail --status-level fail
git diff --check
```

预期：全部 passed；submit tool 不再生成随机 `stage_run_id`。

### 步骤 7：提交

```bash
git add -- backend/crates/golish-db/src/repo/stage_runs.rs backend/crates/golish-db/src/repo/tool_calls.rs backend/crates/golish-db/src/repo/stage_deliverable_submissions.rs backend/crates/golish-db/src/repo/runtime_memory_tx.rs backend/crates/golish-agent-kit/src/db_traits/runtime_memory.rs backend/crates/golish-agent-kit/src/db_traits/tracking.rs backend/crates/golish-agent-kit/src/db_traits/types.rs backend/crates/golish-agent-kit/src/db_tracking/mod.rs backend/crates/golish-agent-kit/src/db_tracking/recording.rs backend/crates/golish-agent-kit/src/db_tracking/types.rs backend/crates/golish-agent-kit/src/task_orchestrator/stage_execution.rs backend/crates/golish-agent-kit/src/task_orchestrator/harness_resume.rs backend/crates/golish-agent-kit/src/task_orchestrator/agent_run_checkpoint.rs backend/crates/golish-agent-kit/src/task_orchestrator/orchestrator.rs backend/crates/golish-agent-kit/src/task_orchestrator/types.rs backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute_harness_loop_tests.rs backend/crates/golish-core/src/agent_session.rs backend/crates/golish-agent-app/src/ai/tracking_bridge/mod.rs backend/crates/golish-agent-app/src/ai/tracking_bridge/records.rs backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs backend/crates/golish-agent-bridge/src/agent_bridge/mod.rs backend/crates/golish-agent-bridge/src/agent_bridge/config.rs backend/crates/golish-agent-bridge/src/agent_bridge/prepare.rs backend/crates/golish-agent-bridge/src/agent_bridge/constructors/mod.rs backend/crates/golish-agent-bridge/src/bridge_executor/trait_impl.rs backend/crates/golish-agent-runtime/src/agentic_loop/context.rs backend/crates/golish-agent-runtime/src/agentic_loop/single_tool_call.rs
git diff --cached --name-only
git commit -m "feat(runtime): bind trusted stage and submission identities"
```

---

## Task 4：持久化 stable scope decision，并原子 freeze Scoping

**文件：** `backend/crates/golish-recon-app/src/organizations/{types.rs,candidates.rs,mod.rs,artifact_cleanup.rs}`、`backend/crates/golish-pentest-app/src/pentest_bridge/manage_organizations.rs`、`frontend/lib/api/organizations.ts`、`frontend/components/AIChatPanel/{ScopeReviewTable.tsx,ScopeReviewTable.test.tsx,AskHumanInline.tsx,AskHumanInline.test.tsx}`、生成的四个 TS 文件、`backend/crates/golish-db/src/repo/{tool_calls.rs,operation_scope_decisions.rs,operation_org_scope.rs,organizations.rs,runtime_memory_tx.rs}`、`backend/crates/golish-agent-kit/src/db_traits/runtime_memory.rs`、`backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/{execute.rs,execute_harness_loop_tests.rs}`。

### 步骤 1：写 scope RED

```rust
#[tokio::test]
async fn exact_execution_decision_freezes_only_selected_stable_ids() {
    let fixture = ScopeFixture::included_children().await;
    fixture.review(vec![
        reviewed(fixture.child_a_candidate(), true),
        reviewed(fixture.child_b_candidate(), false),
    ]).await;
    fixture.promote_reviewed_candidates().await;
    let snapshot = fixture.finalize_scoping().await.unwrap();
    assert_eq!(snapshot.organization_ids(), vec![fixture.root_id(), fixture.child_a_id()]);
    assert_eq!(snapshot.scope_decision_id, fixture.scope_decision_id());
}

#[tokio::test]
async fn edited_name_cannot_rebind_a_candidate_or_foreign_org() {
    let fixture = ScopeFixture::included_children().await;
    fixture.tamper_review_candidate_id("foreign-candidate").await;
    assert!(matches!(fixture.finalize_scoping().await,
        Err(RuntimeMemoryError::IdentityMismatch { code: "scope_decision_row_mismatch" })));
    assert!(fixture.scope_snapshot().await.is_none());
}

#[tokio::test]
async fn freeze_failure_rolls_back_decision_snapshot_unit_submission_and_stage_close() {
    let fixture = ScopeFixture::root_only().await;
    fixture.fail_after_snapshot_header();
    assert!(fixture.finalize_scoping().await.is_err());
    assert!(fixture.scope_decision().await.is_none());
    assert!(fixture.scope_snapshot().await.is_none());
    assert_eq!(fixture.stage_status().await, "started");
}
```

Frontend RED：

```tsx
it("preserves immutable candidate and organization ids while toggling inclusion", async () => {
  const onConfirm = vi.fn();
  render(<ScopeReviewTable kind="unit_review" initial={[unit("cand-1", "org-1")]} onConfirm={onConfirm} onSkip={vi.fn()} />);
  await userEvent.click(screen.getByRole("checkbox", { name: /include/i }));
  await userEvent.click(screen.getByRole("button", { name: /confirm/i }));
  expect(onConfirm).toHaveBeenCalledWith([expect.objectContaining({
    candidateId: "cand-1", organizationId: "org-1", included: false,
  })]);
});
```

### 步骤 2：定义并生成 stable review types

Rust source of truth：

```rust
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct UnitReviewDecisionRow {
    pub review_row_id: String,
    pub candidate_id: String,
    pub organization_id: Option<String>,
    pub name: String,
    pub aliases: Vec<String>,
    pub domains: Vec<String>,
    pub ownership_percent: Option<String>,
    pub included: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../../frontend/lib/generated/")]
pub struct UnitReviewSubmission { pub rows: Vec<UnitReviewDecisionRow> }
```

`OrganizationCandidate.id` 改成必填 TS 字段，并新增 optional `organization_id`、`ownership_percent`。`organization_candidates_list` 还要把已存在 child org 合成为 `existing-org:<uuid>` row，因此 REUSE 路径也有 stable org id。

`unit_review` 改为带 checkbox 的 row state；name/aliases/domains 可编辑但 immutable IDs 不跟随文本重算。用户新增行使用 `crypto.randomUUID()` 作为 `reviewRowId`、空 candidate/org id；backend 只有在同 execution 的 structured create result 将它映射为真实 org 后才接受。`scope_review` 继续保留现有 target textarea语义。

```bash
just gen-types
just check-types
```

预期：生成文件与 Rust 一致；没有手改 `frontend/lib/generated/`。

### 步骤 3：让 proposal/create lifecycle 保留 candidate ID

`manage_organizations(action="create_batch")` 新增结构化 `units`，legacy `names` 仅供 `LegacyV1`：

```json
{
  "action": "create_batch",
  "parent_id": "<root-uuid>",
  "units": [{"review_row_id":"row-1","candidate_id":"cand-1","name":"Child A"}]
}
```

result 必须逐项返回：

```json
{"created":[{"review_row_id":"row-1","candidate_id":"cand-1","organization_id":"<uuid>","name":"Child A"}],"existing":[],"failed":[]}
```

V2 included path 拒绝只含 names 的 batch；proposal、unit review、create mapping 都必须来自同一 `operation_id + stage_execution_id` 的 persisted tool-call rows。

### 步骤 4：实现 server-derived decision 与 freeze transaction

`operation_scope_decisions::derive_exact` 查询 exact execution，而不是 session/time window：

```rust
pub struct ApprovedOrgScopeDecision {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub stage_execution_id: Uuid,
    pub root_organization_id: Uuid,
    pub mode: ApprovedOrgScopeMode,
    pub units: Vec<ApprovedOrgUnit>,
    pub decision_hash: String,
}
```

- root-only：同 execution 的 choice row。
- included：choice → proposal → non-skipped unit review → candidate/org mapping，严格按 tool-call DB order。
- REUSE：review row 直接带 existing org id，仍需本 operation reconfirm。
- CLI：只接受 trusted `CliScopeBootstrap`，不读取 model args。

`scope_hash` canonical JSON key 固定为 `schema_version,project_scope_id,project_path_at_freeze,root_organization_id,mode,units`；units 按 `(depth,ordinal,organization_id)` 排序，ownership 用 decimal string，SHA-256 lowercase hex。

`finalize_scoping_scope` 一个 transaction 完成：验证 trusted submission → derive/insert decision → insert snapshot/units → seed Scoping root unit → 回填 submission unit/org → unit PASS；Scoping `stage_runs` 保持 `started`。随后由 Task 3 的 next-stage entry transaction 原子 close Scoping 并 open successor，避免 freeze 已提交但下一 execution 尚未创建时出现「零 active execution」窗口。相同完整 identity tuple 的 replay 必须幂等返回已 seal 的 decision/snapshot/root unit；任一步失败不推进 cursor。

### 步骤 5：收紧 organization deletion

`organization_delete` 在 artifact cleanup 前调用 runtime port。任一 snapshot unit 命中则返回带 `code=runtime_scope_history_requires_invalidation` 的 conflict；P1 不查询不存在的 cleanup obligation，也不写虚假的 `OrganizationDeleted` event。

### 步骤 6：GREEN 与提交

```bash
pnpm vitest run frontend/components/AIChatPanel/ScopeReviewTable.test.tsx frontend/components/AIChatPanel/AskHumanInline.test.tsx
just check-fe
cd backend && cargo nextest run -p golish-db -E 'test(scope_decision) | test(scope_freeze)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-kit scope_freeze --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-pentest-app manage_organizations --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-recon-app organization_delete --no-tests=fail --status-level fail
git diff --check
```

```bash
git add -- backend/crates/golish-recon-app/src/organizations/types.rs backend/crates/golish-recon-app/src/organizations/candidates.rs backend/crates/golish-recon-app/src/organizations/mod.rs backend/crates/golish-recon-app/src/organizations/artifact_cleanup.rs backend/crates/golish-pentest-app/src/pentest_bridge/manage_organizations.rs frontend/lib/api/organizations.ts frontend/components/AIChatPanel/ScopeReviewTable.tsx frontend/components/AIChatPanel/ScopeReviewTable.test.tsx frontend/components/AIChatPanel/AskHumanInline.tsx frontend/components/AIChatPanel/AskHumanInline.test.tsx frontend/lib/generated/OrganizationCandidate.ts frontend/lib/generated/OrganizationCandidates.ts frontend/lib/generated/UnitReviewDecisionRow.ts frontend/lib/generated/UnitReviewSubmission.ts backend/crates/golish-db/src/repo/tool_calls.rs backend/crates/golish-db/src/repo/operation_scope_decisions.rs backend/crates/golish-db/src/repo/operation_org_scope.rs backend/crates/golish-db/src/repo/organizations.rs backend/crates/golish-db/src/repo/runtime_memory_tx.rs backend/crates/golish-agent-kit/src/db_traits/runtime_memory.rs backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute_harness_loop_tests.rs
git commit -m "feat(scope): freeze exact approved organization identities"
```

---

## Task 5：实现 compound dual-write、Unit/Worker 状态机与故障回滚

**文件：** `backend/crates/golish-db/src/repo/{stage_run_units.rs,stage_worker_runs.rs,message_chains.rs,runtime_memory_tx.rs,stage_asset_waves.rs}`、`backend/crates/golish-agent-kit/src/db_traits/runtime_memory.rs`、`backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs`。

### 步骤 1：写 transaction RED

```rust
#[tokio::test]
async fn every_dual_write_crash_point_rolls_back_both_sources() {
    for crash in [CrashPoint::AfterV2Seed, CrashPoint::AfterLegacyMirror,
                  CrashPoint::AfterV2Checkpoint, CrashPoint::AfterLegacyCheckpoint] {
        let fixture = RuntimeMemoryFixture::dual_write(crash).await;
        assert!(fixture.write_checkpoint(json!({"turn":1})).await.is_err());
        assert_eq!(fixture.v2_checkpoint().await, json!({}));
        assert_eq!(fixture.legacy_checkpoint().await, json!({}));
    }
}

#[tokio::test]
async fn v2_preferred_fallback_selects_one_complete_record() {
    let fixture = RuntimeMemoryFixture::v2_preferred().await;
    fixture.write_v2_record(json!({"turn":2,"chain":"v2"})).await;
    fixture.write_legacy_record(json!({"turn":9,"chain":"legacy"})).await;
    assert_eq!(fixture.load_checkpoint().await.unwrap().source, RuntimeRecordSource::V2);
    fixture.corrupt_v2_record().await;
    let fallback = fixture.load_checkpoint().await.unwrap();
    assert_eq!(fallback.source, RuntimeRecordSource::LegacyFallback);
    assert_eq!(fallback.value, json!({"turn":9,"chain":"legacy"}));
}
```

### 步骤 2：只暴露 compound mutation API

```rust
async fn seed_stage_runtime(&self, input: SeedStageRuntime)
    -> Result<Vec<StageRunUnitView>, RuntimeMemoryError>;
async fn claim_worker_and_bind_chain(&self, input: ClaimWorkerAndBindChain)
    -> Result<ClaimedWorkerView, RuntimeMemoryError>;
async fn checkpoint_worker(&self, input: CheckpointWorker)
    -> Result<WorkerCheckpointView, RuntimeMemoryError>;
async fn begin_worker_tool(&self, input: BeginWorkerTool)
    -> Result<(), RuntimeMemoryError>;
async fn finish_worker_tool(&self, input: FinishWorkerTool)
    -> Result<(), RuntimeMemoryError>;
async fn finish_worker_attempt(&self, input: FinishWorkerAttempt)
    -> Result<(), RuntimeMemoryError>;
```

每个 mutation 在 transaction 内 `SELECT operation_state ... FOR UPDATE` 并读取 frozen contract；caller 不传 read/write mode。`LegacyV1` 只写 JSON；两个 dual states 同事务写 V2 + typed legacy mirror；`V2Only` 只写 V2。独立的 `stage_worker_checkpoint_v2` 与 `operation_state_write_state_blob` 不暴露给 runtime caller。

Unit transition 只允许 `queued→running→{gate_blocked,passed,exhausted}`、`gate_blocked→running`、nonterminal→superseded；所有 transition 带 expected status/row-version。checkpoint WHERE 同时包含 worker/unit/token/epoch/expected version/status，rows affected 0 返回 typed stale/fence error。

### 步骤 3：实现 atomic chain bind

`claim_worker_and_bind_chain` 在一笔 transaction：claim eligible worker、epoch +1、生成 lease、insert 含 initial provider-safe chain JSON 的 `message_chains`、bind unique chain owner、写 initial checkpoint。事务 commit 前不能调用 provider。

### 步骤 4：GREEN 与提交

```bash
cd backend && cargo nextest run -p golish-db -E 'test(dual_write) | test(stage_run_unit) | test(stage_worker) | test(claim_worker_and_bind)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-app runtime_memory_tx --no-tests=fail --status-level fail
git diff --check
```

```bash
git add -- backend/crates/golish-db/src/repo/stage_run_units.rs backend/crates/golish-db/src/repo/stage_worker_runs.rs backend/crates/golish-db/src/repo/message_chains.rs backend/crates/golish-db/src/repo/runtime_memory_tx.rs backend/crates/golish-db/src/repo/stage_asset_waves.rs backend/crates/golish-agent-kit/src/db_traits/runtime_memory.rs backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs
git commit -m "feat(runtime): add atomic unit and worker transactions"
```

---

## Task 6：接入 heartbeat/fencing、prebound chain 与 chat fan-out

**文件：** `backend/crates/golish-agent-runtime/src/agentic_loop/{worker_lease.rs,context.rs,single_tool_call.rs}`、`backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/{stage_run_call.rs,sub_agent_call.rs}`、`backend/crates/golish-sub-agents/src/executor_types.rs`、`backend/crates/golish-sub-agents/src/executor/{chain_persist.rs,inner.rs}`、`backend/crates/golish-agent-app/src/ai/tracking_bridge/chain.rs`、bridge context files。

### 步骤 1：写 fencing RED

```rust
#[tokio::test]
async fn lease_loss_blocks_next_tool_and_stale_landing() {
    let fixture = WorkerFixture::claimed().await;
    fixture.expire_and_reclaim_without_active_tool().await;
    assert!(matches!(fixture.old_worker_dispatch("nmap").await,
        Err(RuntimeMemoryError::LeaseLost { .. })));
    assert!(fixture.old_worker_checkpoint().await.is_err());
}

#[tokio::test]
async fn expired_worker_with_active_tool_requires_manual_recovery() {
    let fixture = WorkerFixture::claimed().await;
    fixture.begin_tool("nmap").await.unwrap();
    fixture.expire_lease().await;
    assert_eq!(fixture.reap().await, WorkerStatus::RecoveryRequired);
    assert!(fixture.reclaim().await.is_err());
}

#[tokio::test]
async fn provider_never_runs_before_prebound_chain_commit() {
    let fixture = WorkerFixture::fail_chain_bind().await;
    assert!(fixture.dispatch().await.is_err());
    assert_eq!(fixture.provider_calls(), 0);
}
```

### 步骤 2：扩展 chain persistence seam

```rust
async fn chain_load_bound_worker(
    &self, worker_run_id: Uuid, chain_id: Uuid, session_id: Uuid, agent_type: &str,
) -> anyhow::Result<Option<Value>>;
async fn chain_checkpoint_bound_worker(
    &self, lease: &WorkerLeaseContext, chain_id: Uuid, chain: &Value,
    expected_checkpoint_version: i64,
) -> anyhow::Result<i64>;
```

ordinary non-stage subagent 保留 `chain_create`；V2 worker 必须拿 `BoundWorkerChainContext`，executor 禁止另建 chain。app implementation 调 Task 5 compound repo，不写 raw SQL。

### 步骤 3：heartbeat 与 dispatch fencing

`WorkerLeaseSupervisor` 每 10 秒 heartbeat；Drop/取消会停止 task。`single_tool_call` 对 worker tool 先持久化 tool-call row，再 `begin_worker_tool`；finish 后清 active marker。未知工具一律按可能有外部副作用处理。lease lost 时停止后续 turn；正在进行的 active tool 若进程中断，由 reaper 置 `recovery_required`，不自动重复。

### 步骤 4：替换 chat live subtree 与 shared result sink

`stage_run_call.rs` 在所有 V2-writing contracts 从 `operation_org_scope_get(operation_id)` seed units；model args 不能增加 org。每 org 解析当前 `ToolExecutionResult.value.response` 中的 local submission id；共享 `RwLock<Option<String>>` 只留 legacy non-stage capture。passed unit 直接 skip；gate-blocked unit resume 同 worker/chain；asset wave 创建新 work item，不创建新 StageRunUnit。

### 步骤 5：GREEN 与提交

```bash
cd backend && cargo nextest run -p golish-agent-runtime -E 'test(lease_loss) | test(active_tool) | test(stage_run_scope) | test(local_submission)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-sub-agents -E 'test(bound_worker) | test(provider_never_runs)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-app chain_bound_worker --no-tests=fail --status-level fail
git diff --check
```

```bash
git add -- backend/crates/golish-agent-runtime/src/agentic_loop/worker_lease.rs backend/crates/golish-agent-runtime/src/agentic_loop/context.rs backend/crates/golish-agent-runtime/src/agentic_loop/single_tool_call.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs backend/crates/golish-sub-agents/src/executor_types.rs backend/crates/golish-sub-agents/src/executor/chain_persist.rs backend/crates/golish-sub-agents/src/executor/inner.rs backend/crates/golish-agent-app/src/ai/tracking_bridge/chain.rs backend/crates/golish-agent-bridge/src/agent_bridge/mod.rs backend/crates/golish-agent-bridge/src/agent_bridge/config.rs backend/crates/golish-agent-bridge/src/agent_bridge/prepare.rs backend/crates/golish-agent-bridge/src/agent_bridge/constructors/mod.rs backend/crates/golish-agent-bridge/src/bridge_executor/trait_impl.rs
git commit -m "feat(runtime): fence and resume stage worker chains"
```

---

## Task 7：实现 closed handoff catalog 与原子 final seal

**文件：** `backend/crates/golish-agent-kit/src/harness/{mod.rs,handoff_catalog.rs}`、`backend/crates/golish-agent-kit/src/harness/gate/{context_builder.rs,rule_engine.rs}`、`backend/crates/golish-db/src/repo/{canonical_fact_refs.rs,stage_handoffs.rs,runtime_memory_tx.rs}`、`backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs`、orchestrator execute/prompts、`stage_run_call.rs`。

### 步骤 1：写 catalog/final-seal RED

```rust
#[tokio::test]
async fn handoff_rejects_unknown_stale_or_foreign_canonical_refs() {
    let fixture = HandoffFixture::passed_gate().await;
    assert!(fixture.build_with_foreign_org_ref().await.is_err());
    assert!(fixture.build_with_stale_ref().await.is_err());
    assert!(fixture.build_with_unknown_kind().await.is_err());
}

#[tokio::test]
async fn unit_pass_handoff_and_compat_projection_are_atomic() {
    let fixture = HandoffFixture::running_unit().await;
    fixture.fail_after_handoff_insert();
    assert!(fixture.final_seal().await.is_err());
    assert_eq!(fixture.unit_status().await, UnitStatus::Running);
    assert!(fixture.handoffs().await.is_empty());
    assert!(fixture.compat_completion().await.is_none());
}
```

### 步骤 2：完成 catalog

在 §1.4 enum 加 `Finding { finding_id: Uuid }`。`handoff_catalog` 为每个 key 定义 exact repo query 与 freshness/ownership check；`TechniqueOutcome` 使用其自然唯一 key和 row content hash。GateContext builder 增加 canonical source hints，但最终 ID、timestamp、hash 都由 server repo 回查，不从 deliverable claim 接受。

### 步骤 3：实现 bounded server builder 与 final seal

```rust
pub struct StageHandoffPayload {
    pub canonical_fact_refs: Vec<CanonicalFactRef>,
    pub typed_claims: Vec<TypedHandoffClaim>,
    pub coverage_watermark: Value,
    pub evidence_ids: Vec<i64>,
}
```

没有 public model-controlled constructor。`finalize_unit_pass` 一个 transaction 验证 submission、lease/epoch、scope hash、Gate decision hash、catalog refs和 evidence ownership，然后写 unit PASS + handoff + `org_stage_completions`。wave completion只写 worker/watermark，不发布 handoff。下游按 `StageSpec.inherits_evidence_from` 读取同 operation/org 最新 final-sealed handoff。

### 步骤 4：GREEN 与提交

```bash
cd backend && cargo nextest run -p golish-agent-kit handoff_catalog --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-db -E 'test(stage_handoff) | test(final_seal)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-runtime inherited_handoff --no-tests=fail --status-level fail
git diff --check
```

```bash
git add -- backend/crates/golish-agent-kit/src/harness/mod.rs backend/crates/golish-agent-kit/src/harness/handoff_catalog.rs backend/crates/golish-agent-kit/src/harness/gate/context_builder.rs backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs backend/crates/golish-db/src/repo/canonical_fact_refs.rs backend/crates/golish-db/src/repo/stage_handoffs.rs backend/crates/golish-db/src/repo/runtime_memory_tx.rs backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs backend/crates/golish-agent-kit/src/task_orchestrator/prompts/mod.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs
git commit -m "feat(harness): seal canonical stage handoffs atomically"
```

---

## Task 8：迁移 CLI 单 operation、resume/reaper、dev reset 与诊断

**文件：** `backend/crates/golish/src/stage_run/{mod.rs,runtime_v2.rs,fleet.rs,scheduler.rs}`、`backend/crates/golish-db/src/repo/{tasks.rs,runtime_memory_tx.rs}`、`backend/crates/golish-agent-app/src/ai/commands/harness_dev.rs`、`scripts/{run_tree.py,tests/test_run_tree_runtime_memory.py}`。

### 步骤 1：写 CLI/recovery RED

```rust
#[tokio::test]
async fn cli_descendants_share_one_operation_and_snapshot() {
    let fixture = CliFixture::root_and_two_children().await;
    let result = fixture.run_v2_cli().await.unwrap();
    assert_eq!(result.operation_ids.len(), 1);
    assert_eq!(result.scope_unit_count, 3);
    assert_eq!(result.stage_unit_count, 3);
}

#[test]
fn resumability_distinguishes_scoping_specialist_and_root_only_units() {
    assert_eq!(classify(v2_scoping_before_freeze()), ResumeDecision::ResumeScoping);
    assert_eq!(classify(v2_specialist_with_live_lease()), ResumeDecision::WaitForLease);
    assert_eq!(classify(v2_non_specialist_root_unit()), ResumeDecision::ResumeRootUnit);
    assert_eq!(classify(expired_active_tool()), ResumeDecision::RecoveryRequired);
}
```

### 步骤 2：CLI one-operation bootstrap

`runtime_v2.rs` 一次解析 flags、注册 project scope、解析 root/descendants/51% threshold、创建一个 operation、写 `CliFlags` decision/snapshot，再调用一次 `run_stage`。V2-writing contract 禁止 `OrgFleetExecutor` 创建 child task/operation；`fleet.rs` 当前 per-org operation path 仅保留 `LegacyV1` fallback，`scheduler.rs` 加 regression test证明 V2 adapter不会调用它。report 从 snapshot/unit rows 聚合。

### 步骤 3：按 contract 重写 reaper

- `LegacyV1` 使用现有 JSON predicate。
- V2 Scoping pre-freeze 允许 exact active stage execution、无 snapshot/unit。
- specialist stage 要求 snapshot + units；unexpired lease保持 waiting。
- expired lease + no active tool 可 requeue；expired + active tool转 `recovery_required`。
- post-scope non-specialist root unit可没有 WorkerRun。
- malformed/cross-op identity 一律 fail closed。

### 步骤 4：dev reset supersede，不留下 stale V2 state

`harness_dev_reset_stage_checkpoint` 调 compound repo：终止 lease、将受影响 worker/unit/stage execution标 `superseded`、invalidate handoff、重置 cursor；按 frozen contract同步 legacy mirror。普通 reset 不删除 runtime history；`restart_from_stage_purge` 保留既有显式 fact purge语义。

### 步骤 5：run_tree 输出与测试

`--db` 增加 rollout/operation contract、scope decision/hash/units、stage execution/unit、worker lease/epoch/active tool/chain/checkpoint、submission、handoff、selected read source/legacy fallback。Python fixture逐字段断言，并覆盖 cross-org rejection。

```bash
cd backend && cargo nextest run -p golish -E 'test(cli_descendants) | test(resumability)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-db startup_reaper --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-app harness_dev --no-tests=fail --status-level fail
python3 -m py_compile scripts/run_tree.py
python3 -m unittest scripts.tests.test_run_tree_runtime_memory
git diff --check
```

```bash
git add -- backend/crates/golish/src/stage_run/mod.rs backend/crates/golish/src/stage_run/runtime_v2.rs backend/crates/golish/src/stage_run/fleet.rs backend/crates/golish/src/stage_run/scheduler.rs backend/crates/golish-db/src/repo/tasks.rs backend/crates/golish-db/src/repo/runtime_memory_tx.rs backend/crates/golish-agent-app/src/ai/commands/harness_dev.rs scripts/run_tree.py scripts/tests/test_run_tree_runtime_memory.py
git commit -m "feat(runtime): converge CLI and recovery on V2 operations"
```

---

## Task 9：声明四 stage contract、执行生产 cutover、同步文档与证据

**文件：** `backend/crates/golish-agent-kit/src/harness/{mod.rs,stage_spec.rs,stage_runtime_contract.rs}`、四份 stage spec、`20260712000002_runtime_memory_v2_cutover.sql`、全部相关模块卡、`docs/modules/INDEX.md`、`agent-progress.md`、`feature_list.json`。

### 步骤 1：用 StageSpec 消除未归属 API

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageRuntimeContract {
    pub schema_version: u16,
    pub unit_identity: RuntimeUnitIdentity,
    pub scope_source: RuntimeScopeSource,
    pub requires_worker_lease: bool,
    pub publishes_handoff_after_final_seal: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeUnitIdentity { StageExecutionOrganization }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeScopeSource { FrozenOperationSnapshot }
```

`StageSpec` 新增 `runtime_memory: Option<StageRuntimeContract>`。四个 specialist spec逐字写：

```json
"runtime_memory": {
  "schema_version": 2,
  "unit_identity": "stage_execution_organization",
  "scope_source": "frozen_operation_snapshot",
  "requires_worker_lease": true,
  "publishes_handoff_after_final_seal": true
}
```

参数化测试从 `load_embedded_stage_spec(stage).runtime_memory` 读取，不使用未定义的 `runtime_contract_for`。

### 步骤 2：四 stage 逐一 V2 acceptance

对 `target_intel`、`external_attack_surface`、`enumeration`、`vuln_triage` 各跑：snapshot fanout、独立 worker、lease loss、local submission、final handoff、restart skip passed org。全部是 mock/embedded DB，不发真实 LLM、扫描或外部请求。

### 步骤 3：创建 forward-only cutover migration

只有步骤 2 全绿后创建 `20260712000002_runtime_memory_v2_cutover.sql`：

```sql
UPDATE runtime_memory_rollout SET contract='dual_write_legacy_read', contract_rank=1,
 row_version=row_version+1, updated_at=NOW()
WHERE singleton_id=1 AND contract='legacy_v1' AND contract_rank=0;
UPDATE runtime_memory_rollout SET contract='dual_write_v2_preferred', contract_rank=2,
 row_version=row_version+1, updated_at=NOW()
WHERE singleton_id=1 AND contract='dual_write_legacy_read' AND contract_rank=1;
UPDATE runtime_memory_rollout SET contract='v2_only', contract_rank=3,
 row_version=row_version+1, updated_at=NOW()
WHERE singleton_id=1 AND contract='dual_write_v2_preferred' AND contract_rank=2;
DO $$ BEGIN
  IF NOT EXISTS (SELECT 1 FROM runtime_memory_rollout WHERE singleton_id=1 AND contract='v2_only' AND contract_rank=3) THEN
    RAISE EXCEPTION 'runtime memory cutover did not reach v2_only';
  END IF;
END $$;
```

existing operation 的 frozen contract不变；rollback不能 downgrade singleton，只能部署仍理解 V2 的修复版本或停止创建新 operation。

### 步骤 4：更新完整模块卡

同步以下卡和 INDEX 状态：`golish-db{.md,/repo.md}`、`golish-agent-kit{.md,/db_tracking.md,/db_traits.md,/harness.md,/task_orchestrator.md}`、`golish-agent-app{.md,/ai.md}`、`golish-agent-bridge{.md,/agent_bridge.md,/bridge_executor.md}`、`golish-agent-runtime{.md,/agentic_loop.md}`、`golish-sub-agents{.md,/executor.md}`、`golish{.md,/stage_run.md}`、`golish-recon-app/organizations.md`、`golish-pentest-app/pentest_bridge.md`、`frontend/components.md`。

### 步骤 5：全量验证与状态更新

```bash
just gen-types
just check-types
just check-fe
just test-fe
cd backend && cargo nextest run -p golish-db -p golish-agent-kit -p golish-agent-app -p golish-agent-runtime -p golish-sub-agents -p golish-agent-bridge -p golish --status-level fail
cd backend && cargo clippy -p golish-db -p golish-agent-kit -p golish-agent-app -p golish-agent-runtime -p golish-sub-agents -p golish-agent-bridge -p golish --all-targets -- -D warnings
cd backend && cargo fmt --all -- --check
python3 -m unittest scripts.tests.test_run_tree_runtime_memory
git diff --check
just precommit
```

预期全部 exit 0。把命令、exit code、关键输出逐条复制到 `agent-progress.md`；按 `feature_list.json.verification` 填 evidence。只有 AGENTS.md 五项完成定义全部满足才能标 `passing`。

### 步骤 6：最终提交

```bash
git add -- backend/crates/golish-agent-kit/src/harness/mod.rs backend/crates/golish-agent-kit/src/harness/stage_spec.rs backend/crates/golish-agent-kit/src/harness/stage_runtime_contract.rs resources/harness/stages/target_intel/spec.json resources/harness/stages/external_attack_surface/spec.json resources/harness/stages/enumeration/spec.json resources/harness/stages/vuln_triage/spec.json backend/crates/golish-db/migrations/20260712000002_runtime_memory_v2_cutover.sql docs/modules/INDEX.md docs/modules/backend/golish-db.md docs/modules/backend/golish-db/repo.md docs/modules/backend/golish-agent-kit.md docs/modules/backend/golish-agent-kit/db_tracking.md docs/modules/backend/golish-agent-kit/db_traits.md docs/modules/backend/golish-agent-kit/harness.md docs/modules/backend/golish-agent-kit/task_orchestrator.md docs/modules/backend/golish-agent-app.md docs/modules/backend/golish-agent-app/ai.md docs/modules/backend/golish-agent-bridge.md docs/modules/backend/golish-agent-bridge/agent_bridge.md docs/modules/backend/golish-agent-bridge/bridge_executor.md docs/modules/backend/golish-agent-runtime.md docs/modules/backend/golish-agent-runtime/agentic_loop.md docs/modules/backend/golish-sub-agents.md docs/modules/backend/golish-sub-agents/executor.md docs/modules/backend/golish.md docs/modules/backend/golish/stage_run.md docs/modules/backend/golish-recon-app/organizations.md docs/modules/backend/golish-pentest-app/pentest_bridge.md docs/modules/frontend/components.md agent-progress.md feature_list.json
git diff --cached --name-only
git commit -m "feat(runtime): cut new operations over to runtime memory V2"
```

---

## 3. P1 完成后明确不做

- 不实现 CandidateApproval/AttackAttempt/FactDelta；P2 只消费本计划的 `scope_snapshot_id`、`stage_run_unit_id`、`stage_execution_id`、`deliverable_submission_id`。
- 不创建 assertion/vector/KG/outbox、cleanup obligation 或 reporting revision。
- 不删除 legacy JSON、`org_stage_completions` 或旧 fleet code；删除走单独 contract migration。
- 不自动执行真实 LLM、扫描、exploit 或外部 API smoke。
- 不借本计划清理 `scripts/check_repo_ownership.py` 在 `ab7b0c4a` 已存在的历史 violations；只证明 P1 没有新增 violation 且所有新 repo 已注册。
