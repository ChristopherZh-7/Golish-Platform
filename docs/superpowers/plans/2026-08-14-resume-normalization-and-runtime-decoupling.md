# Resume 规范化与运行时解耦实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 把 GUI、Candidate Review、CLI 和 startup reaper 的 resume 收敛为一个带协议版本、单快照、纯分类、幂等修复、原子 claim ticket 和 typed halt 的恢复状态机，使重复“继续”只推进确定性状态或返回稳定 blocker，绝不误开新任务、盲重放工具或把可恢复 blocker 标成 terminal failure。

**架构：** `golish-db` 在一个 repeatable-read transaction 中生成 `ResumeSnapshot`，纯 classifier 只返回闭集 `ResumeDisposition`，normalizer 以最多八次 CAS-safe repair 达到 fixed point，随后 DB 用 snapshot fingerprint + open Turn witness 原子签发一次性 `ClaimedExactResume`。GUI、Candidate Review 和 CLI 只保留 adapter-specific authorization；`stage_run` 以同一 typed control/fingerprint 停止本 request，provider dispatch 与 DB repair 成为互斥边界。

**技术栈：** Rust 2021、Tokio、SQLx/PostgreSQL、Tauri 2、serde/serde_json、sha2、cargo-nextest、embedded Postgres、Vitest。

## 执行前置条件与授权边界

- 当前唯一 `in_progress` 功能 `investigation-primary-led-verification-execution-2026-08-12` 先完成、转为 `passing|blocked`，或由用户明确要求暂停；本计划登记为 `not_started`，不得抢占它。
- Task 2 会新增 forward migration，Task 10 会删旧执行兼容代码。执行这两项前必须在聊天中取得用户对 schema/migration 与删除代码的明确确认。
- 不用当前 failed retained Investigation operation 作为 GREEN fixture。协议 gate 上线后，旧未版本化 active stage 应得到 `UNSUPPORTED_FROZEN_CONTRACT`，需要从 final-sealed predecessor fork 新 stage。
- 不运行 `./init.sh`、`just check`、`just precommit` 或全 workspace suite，除非用户在执行该任务时明确授权。每个 Cargo 命令前先运行 `just space-guard`。
- 不修改旧 migration；不删除 evidence、receipt、Turn、tool、worker、chain 或历史 projection。

## 文件结构

### 新建

- `backend/crates/golish-core/src/resume.rs`：resume protocol/source/blocker/retry/runtime-control 类型与稳定 fingerprint。
- `backend/crates/golish-db/migrations/20260814000011_stage_run_resume_contract.sql`：stage execution 的 immutable behavior protocol。
- `backend/crates/golish-db/src/repo/runtime_resume/mod.rs`：DB resume authority facade。
- `backend/crates/golish-db/src/repo/runtime_resume/authority.rs`：唯一 candidate/source/recoverability predicate。
- `backend/crates/golish-db/src/repo/runtime_resume/snapshot.rs`：repeatable-read snapshot 与 fingerprint material。
- `backend/crates/golish-db/src/repo/runtime_resume/claim.rs`：fingerprint/open-Turn CAS 与 opaque claim ticket。
- `backend/crates/golish-db/src/repo/runtime_resume/repair.rs`：closed allowlist、receipt-backed、无外部 I/O 的修复。
- `backend/crates/golish-db/src/repo/runtime_resume/reaper.rs`：复用同一 authority/repair 的 startup reconciliation。
- `backend/crates/golish-db/src/repo/runtime_resume/tool_fence.rs`：tool row + WorkerRun active-tool 原子 begin/finish。
- `backend/crates/golish-db/tests/runtime_resume_snapshot.rs`：snapshot、claim、source、Turn、post-synthesis 行为测试。
- `backend/crates/golish-db/tests/runtime_resume_reaper.rs`：selector/reaper 同义性与 repair rollback 测试。
- `backend/crates/golish-db/tests/runtime_tool_lifecycle.rs`：工具 begin/finish 与 outcome-unknown 测试。
- `backend/crates/golish-agent-app/src/ai/resume/mod.rs`：应用层公共 facade。
- `backend/crates/golish-agent-app/src/ai/resume/classify.rs`：纯 common classifier。
- `backend/crates/golish-agent-app/src/ai/resume/investigation.rs`：Investigation resume contract classifier。
- `backend/crates/golish-agent-app/src/ai/resume/service.rs`：bounded normalization 与 coordinator。
- `backend/crates/golish-agent-app/tests/exact_resume_coordinator.rs`：GUI/Candidate/CLI 可复用 coordinator contract tests。
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run/mod.rs`：stage-run facade。
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run/snapshot.rs`：runtime snapshot adapter。
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run/classify.rs`：纯 Stage/Company Unit action classifier。
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run/repair.rs`：typed DB repair adapter。
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run/dispatch.rs`：唯一 provider/semaphore 边界。
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run/control.rs`：typed halt JSON projection。

### 修改

- `backend/crates/golish-core/src/lib.rs`：导出 resume contracts。
- `backend/crates/golish-db/src/repo/mod.rs`：导出 `runtime_resume`。
- `backend/crates/golish-db/src/repo/stage_runs.rs`：读写 immutable `resume_contract`。
- `backend/crates/golish-db/src/repo/tasks.rs`：保留 task CRUD；旧 resume API 暂作 facade 后删除重复 SQL。
- `backend/crates/golish-db/src/repo/message_chains.rs`：CRUD/CAS 留下，exact-resume projection 迁入 snapshot。
- `backend/crates/golish-db/src/repo/tool_calls.rs`：通用审计读写留下，worker-fenced 生命周期迁入 `tool_fence`。
- `backend/crates/golish-db/src/repo/runtime_memory_tx.rs`：保留兼容 facade，迁出 resume/reaper/tool-fence 实现。
- `backend/crates/golish-agent-app/src/ai/mod.rs`：导出应用层 resume service。
- `backend/crates/golish-agent-app/src/ai/commands/core/operation_resume.rs`：只保留 command adapter 与 authorization。
- `backend/crates/golish-agent-app/src/ai/commands/core/chat.rs`：DB error fail closed；仅 `NoCandidate` 允许 fresh。
- `backend/crates/golish-agent-app/src/ai/commands/attack.rs`：Candidate barrier 后调用 coordinator。
- `backend/crates/golish-agent-kit/src/task_orchestrator/orchestrator.rs`：新增 consuming `resume_claimed`，移除 unfenced production path。
- `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`：从 ticket 注入唯一 source/Turn/contract。
- `backend/crates/golish-agent-bridge/src/agent_bridge/config.rs`、`prepare.rs`、`mod.rs`：停止独立 mutable resume source setter。
- `backend/crates/golish-agent-runtime/src/agentic_loop/context.rs`：reentry guard 保存 typed halt token。
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_dispatch.rs`：只反序列化 typed runtime control。
- `backend/crates/golish-agent-runtime/src/agentic_loop/worker_tool_lifecycle.rs`：调用 DB atomic tool-fence API。
- `backend/crates/golish-agent-kit/src/db_traits/runtime_memory.rs`：向 runtime port 暴露原子 begin/finish DTO 与方法。
- `backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs`：把 runtime port 委托到 DB atomic tool-fence repository。
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`：行为锁定后按 seam 迁出代码。
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_team_scheduler.rs`：仅保留 plan/compiler/hash/parser。
- `backend/crates/golish/src/stage_run/mod.rs`：CLI 使用 shared coordinator/ticket。
- `backend/crates/golish/src/stage_run/runtime_v2.rs`：删除第二套 canonical loader/classifier，只留 CLI view/diagnostic helpers。
- `docs/modules/backend/golish-core.md`、`golish-db.md`、`golish-db/repo.md`、`golish-agent-app/ai.md`、`golish-agent-kit/task_orchestrator.md`、`golish-agent-runtime.md`、`golish-agent-runtime/agentic_loop.md`、`golish/stage_run.md`、`docs/modules/INDEX.md`：同步新职责与入口。

## Task 1：先冻结 resume domain vocabulary

**文件：**

- 创建 `backend/crates/golish-core/src/resume.rs`
- 修改 `backend/crates/golish-core/src/lib.rs`

**步骤 1：写 protocol、source、blocker 和 control 的失败单测。**

在 `resume.rs` 的 `#[cfg(test)]` 中先写：

```rust
#[test]
fn unversioned_contract_is_parseable_but_never_runnable() {
    let contract = StageResumeContract::try_parse("legacy_unversioned_v1").unwrap();
    assert_eq!(contract, StageResumeContract::LegacyUnversionedV1);
    assert!(!contract.is_runnable());
}

#[test]
fn blocker_fingerprint_excludes_free_form_detail() {
    let left = ResumeBlockerMaterial::fixture("first prose");
    let right = ResumeBlockerMaterial::fixture("different prose");
    assert_eq!(left.fingerprint(), right.fingerprint());
}

#[test]
fn halt_does_not_imply_retry_fuel_exhausted() {
    let control = StageRunRuntimeControl::halt(
        StageRunHaltReason::RuntimeRecovered,
        RetryDisposition::NewRequest,
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        Vec::new(),
    );
    assert_ne!(control.retry, RetryDisposition::Exhausted);
}
```

运行：

```bash
just space-guard
cd backend && cargo nextest run -p golish-core -E 'test(unversioned_contract_is_parseable_but_never_runnable) | test(blocker_fingerprint_excludes_free_form_detail) | test(halt_does_not_imply_retry_fuel_exhausted)' --status-level fail
```

预期：编译因类型不存在而失败，证明测试先于实现。

**步骤 2：实现闭集类型。**

核心定义固定为：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageResumeContract {
    LegacyUnversionedV1,
    DurableStageRunV1,
    InvestigationAssetPrimaryDynamicV2,
}

impl StageResumeContract {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LegacyUnversionedV1 => "legacy_unversioned_v1",
            Self::DurableStageRunV1 => "durable_stage_run_v1",
            Self::InvestigationAssetPrimaryDynamicV2 => {
                "investigation_asset_primary_dynamic_v2"
            }
        }
    }

    pub const fn current_for_stage(stage_kind: &str) -> Self {
        if matches!(stage_kind, "investigation") {
            Self::InvestigationAssetPrimaryDynamicV2
        } else {
            Self::DurableStageRunV1
        }
    }

    pub const fn is_runnable(self) -> bool {
        !matches!(self, Self::LegacyUnversionedV1)
    }

    pub fn try_parse(value: &str) -> Result<Self, ResumeContractParseError> {
        match value {
            "legacy_unversioned_v1" => Ok(Self::LegacyUnversionedV1),
            "durable_stage_run_v1" => Ok(Self::DurableStageRunV1),
            "investigation_asset_primary_dynamic_v2" => {
                Ok(Self::InvestigationAssetPrimaryDynamicV2)
            }
            _ => Err(ResumeContractParseError(value.to_string())),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unsupported stage resume contract: {0}")]
pub struct ResumeContractParseError(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResumeRecordSource {
    Legacy,
    V2,
    LegacyFallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryDisposition {
    None,
    NewRequest,
    AfterOperatorResolution,
    Exhausted,
}
```

`ResumeBlockerMaterial` 的 hash material 只含 operation/stage execution/contract/source/open Turn、
entity row versions、tool fence、blocker code 和 authoritative subject refs；`detail` 只用于展示，
不得进入序列化 hash struct。`StageRunRuntimeControl` 使用 `#[serde(deny_unknown_fields)]`。

**步骤 3：导出并跑 GREEN。**

在 `golish-core/src/lib.rs` 加：

```rust
pub mod resume;
pub use resume::{
    ResumeBlocker, ResumeRecordSource, RetryDisposition, StageResumeContract,
    StageRunHaltReason, StageRunRuntimeControl,
};
```

重复步骤 1 的 nextest。预期 exit 0，三个测试全部 PASS。

**验证：**

```bash
just space-guard
cd backend && cargo clippy -p golish-core --lib -- -D warnings
cargo fmt --manifest-path backend/Cargo.toml --all -- --check
```

预期：两条命令 exit 0，无 warning。

**提交：**

```bash
git add backend/crates/golish-core/src/resume.rs backend/crates/golish-core/src/lib.rs
git commit -m "feat(resume): define durable resume control contracts"
```

## Task 2：冻结每个 stage execution 的 behavior protocol

> 执行本 Task 前先取得用户对 forward migration 的明确确认。

**文件：**

- 创建 `backend/crates/golish-db/migrations/20260814000011_stage_run_resume_contract.sql`
- 修改 `backend/crates/golish-db/src/repo/stage_runs.rs`
- 创建 `backend/crates/golish-db/tests/runtime_resume_protocol.rs`

**步骤 1：写 migrated-PG RED tests。**

测试必须覆盖以下断言：

```rust
assert_eq!(historical.resume_contract, "legacy_unversioned_v1");
assert_eq!(fresh_investigation.resume_contract, "investigation_asset_primary_dynamic_v2");
assert_eq!(fresh_scoping.resume_contract, "durable_stage_run_v1");
assert!(sqlx::query("UPDATE stage_runs SET resume_contract='durable_stage_run_v1' WHERE id=$1")
    .bind(fresh_investigation.id)
    .execute(db.pool())
    .await
    .is_err());
```

运行：

```bash
just space-guard
cd backend && cargo nextest run -p golish-db --test runtime_resume_protocol --status-level fail
```

预期：新列/类型不存在导致 RED。

**步骤 2：新增 forward-only schema。**

Migration 使用以下 SQL contract：

```sql
ALTER TABLE stage_runs ADD COLUMN resume_contract TEXT;

UPDATE stage_runs
   SET resume_contract = 'legacy_unversioned_v1';

ALTER TABLE stage_runs
    ALTER COLUMN resume_contract SET NOT NULL,
    ADD CONSTRAINT stage_runs_resume_contract_check CHECK (
        resume_contract IN (
            'legacy_unversioned_v1',
            'durable_stage_run_v1',
            'investigation_asset_primary_dynamic_v2'
        )
        AND (
            resume_contract = 'legacy_unversioned_v1'
            OR (stage_kind = 'investigation'
                AND resume_contract = 'investigation_asset_primary_dynamic_v2')
            OR (stage_kind <> 'investigation'
                AND resume_contract = 'durable_stage_run_v1')
        )
    );

CREATE FUNCTION guard_stage_run_resume_contract()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.resume_contract IS DISTINCT FROM OLD.resume_contract THEN
        RAISE EXCEPTION 'STAGE_RUN_RESUME_CONTRACT_IMMUTABLE' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER stage_runs_resume_contract_immutable
BEFORE UPDATE OF resume_contract ON stage_runs
FOR EACH ROW EXECUTE FUNCTION guard_stage_run_resume_contract();

CREATE FUNCTION reject_new_unversioned_stage_run()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.resume_contract = 'legacy_unversioned_v1' THEN
        RAISE EXCEPTION 'NEW_STAGE_RUN_REQUIRES_VERSIONED_RESUME_CONTRACT' USING ERRCODE='23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER stage_runs_reject_new_unversioned
BEFORE INSERT ON stage_runs
FOR EACH ROW EXECUTE FUNCTION reject_new_unversioned_stage_run();
```

不设置 DB default。任何绕过 repository 的新 insert 若未显式给 contract 必须失败。

**步骤 3：repository 显式写 current contract。**

`StageRunRow`、`STAGE_RUN_COLUMNS` 和所有显式 SELECT 加 `resume_contract`。Insert 固定为：

```rust
let resume_contract = golish_core::StageResumeContract::current_for_stage(stage_kind);
let sql = format!(
    "INSERT INTO stage_runs (id, operation_id, stage_kind, resume_contract) \
     VALUES ($1, $2, $3, $4) RETURNING {STAGE_RUN_COLUMNS}"
);
```

测试 fixture 的 struct literal 全部增加准确字段；生产调用方不自行选择 protocol。

**步骤 4：跑 GREEN 和 migration shape checks。**

```bash
just space-guard
cd backend && cargo nextest run -p golish-db --test runtime_resume_protocol --status-level fail
just space-guard
cd backend && cargo nextest run -p golish-db -E 'test(stage_run_row_serde_roundtrip) | test(typed_stage_execution_terminal_values_match_database_contract)' --status-level fail
```

预期：exit 0；fresh/historical/immutable 四类断言 PASS。

**验证：**

```bash
just space-guard
cd backend && cargo clippy -p golish-db --test runtime_resume_protocol -- -D warnings
git diff --check
```

预期：exit 0，无 warning/whitespace error。

**提交：**

```bash
git add backend/crates/golish-db/migrations/20260814000011_stage_run_resume_contract.sql backend/crates/golish-db/src/repo/stage_runs.rs backend/crates/golish-db/tests/runtime_resume_protocol.rs
git commit -m "feat(resume): freeze stage execution resume protocol"
```

## Task 3：建立唯一 resume authority predicate

**文件：**

- 创建 `backend/crates/golish-db/src/repo/runtime_resume/mod.rs`
- 创建 `backend/crates/golish-db/src/repo/runtime_resume/authority.rs`
- 修改 `backend/crates/golish-db/src/repo/mod.rs`
- 修改 `backend/crates/golish-db/src/repo/tasks.rs`
- 创建 `backend/crates/golish-db/tests/runtime_resume_reaper.rs`

**步骤 1：先写 selector/reaper 语义漂移 RED。**

构造一个 post-synthesis V2 checkpoint，断言：

```rust
let reaped = runtime_resume::reaper::reconcile_operation(db.pool(), operation_id).await.unwrap();
assert_eq!(reaped.task_status(), TaskStatus::Waiting);

let candidate = runtime_resume::authority::latest_candidate(db.pool(), session_id)
    .await
    .unwrap()
    .expect("reaper-preserved checkpoint must be discoverable");
assert_eq!(candidate.operation_id, operation_id);
assert_eq!(candidate.source, ResumeRecordSource::V2);
```

另加 `database_error_is_not_none`：关闭 embedded PG 后调用 `latest_candidate`，断言 `Err`。

运行：

```bash
just space-guard
cd backend && cargo nextest run -p golish-db --test runtime_resume_reaper --status-level fail
```

预期：post-synthesis 用例 RED，或证明当前 selector/reaper 已一致；DB error 用例必须确认 error 不被吞。

**步骤 2：只保留一份 recoverability SQL。**

`authority.rs` 提供：

```rust
pub async fn latest_candidate(
    pool: &PgPool,
    session_id: Uuid,
) -> RuntimeMemoryStoreResult<Option<ResumeCandidate>>;

pub async fn select_source(
    executor: impl Executor<'_, Database = Postgres>,
    operation_id: Uuid,
    session_id: Uuid,
) -> RuntimeMemoryStoreResult<Option<ResumeRecordSource>>;
```

把 `RECOVERABLE_ABANDONED_CHECKPOINT_SQL`、普通 V2 和 post-synthesis V2 条件移入一个 private
SQL fragment。`tasks::latest_resumable_by_session` 与 `tasks::exact_resumable_runtime_source` 在本
Task 只作为兼容 facade 委托，不复制 SQL。startup reaper 也引用该 fragment。

**步骤 3：锁住 source policy 与安全边界。**

新增或迁移这些断言：

```rust
assert_eq!(v2_only_with_legacy_blob.source, ResumeRecordSource::V2);
assert!(live_lease_candidate.is_none());
assert!(wrong_session_candidate.is_none());
assert!(terminal_task_candidate.is_none());
```

Legacy/Dual/Fallback 仍由单独 `source_policy` match 支持；本 Task 不删除它们。

**验证：**

```bash
just space-guard
cd backend && cargo nextest run -p golish-db --test runtime_resume_reaper --status-level fail
just space-guard
cd backend && cargo nextest run -p golish-db -E 'test(exact_resume_source_prefers_one_complete_v2_record_before_legacy_fallback) | test(post_synthesis_reaper_requires_exact_expired_current_primary_authority) | test(post_synthesis_recovery_is_shared_by_task_pause_and_fail_complement)' --status-level fail
```

预期：exit 0，reaper-preserved 状态可由 ordinary selector 发现，现有 source/security tests 不回归。

**提交：**

```bash
git add backend/crates/golish-db/src/repo/runtime_resume backend/crates/golish-db/src/repo/mod.rs backend/crates/golish-db/src/repo/tasks.rs backend/crates/golish-db/tests/runtime_resume_reaper.rs
git commit -m "refactor(resume): centralize recoverability authority"
```

## Task 4：单快照读取与 fingerprint claim ticket

**文件：**

- 创建 `backend/crates/golish-db/src/repo/runtime_resume/snapshot.rs`
- 创建 `backend/crates/golish-db/src/repo/runtime_resume/claim.rs`
- 修改 `backend/crates/golish-db/src/repo/runtime_resume/mod.rs`
- 修改 `backend/crates/golish-db/src/repo/message_chains.rs`
- 修改 `backend/crates/golish-db/src/repo/tasks.rs`
- 创建 `backend/crates/golish-db/tests/runtime_resume_snapshot.rs`

**步骤 1：写五个 DB RED tests。**

测试名与核心断言固定为：

```rust
snapshot_reads_turn_source_workers_chains_and_tools_from_one_repeatable_read_view();
snapshot_fingerprint_drift_makes_claim_zero_write();
cross_session_or_wrong_agent_chain_fails_before_claim();
two_claimers_same_snapshot_only_one_advances_turn();
unsupported_contract_claim_is_zero_write();
```

每个 zero-write 用例都断言：prior Turn 仍 open、successor count=0、task status/result 不变、
Controller successor receipt 未新增。

**步骤 2：实现最小 snapshot types。**

`snapshot.rs` 使用以下 shape：

```rust
pub struct ResumeSnapshot {
    pub operation_id: Uuid,
    pub session_id: Uuid,
    pub source: ResumeRecordSource,
    pub resume_contract: StageResumeContract,
    pub project_scope_id: Uuid,
    pub profile: String,
    pub current_stage: String,
    pub open_turn: ResumeTurnSnapshot,
    pub stage_execution: ResumeStageExecutionSnapshot,
    pub lanes: Vec<ResumeLaneSnapshot>,
    pub fingerprint: String,
}

pub async fn select_resume_snapshot(
    pool: &PgPool,
    operation_id: Uuid,
    session_id: Uuid,
) -> RuntimeMemoryStoreResult<Option<ResumeSnapshot>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    let snapshot = load_snapshot_in_transaction(&mut tx, operation_id, session_id).await?;
    tx.commit().await?;
    Ok(snapshot)
}
```

Lane 只存可判定与可 CAS 的 ID/status/row version/attempt epoch/checkpoint version/lease/tool tuple。
Chain body 作为 JSON 返回，fingerprint 只含 chain SHA-256。

**步骤 3：实现 opaque ticket。**

`claim.rs` 中 fields private，只暴露 getter：

```rust
pub struct ClaimedExactResume {
    operation_id: Uuid,
    session_id: Uuid,
    source: ResumeRecordSource,
    prior_turn_id: Uuid,
    successor_turn_id: Uuid,
    resume_contract: StageResumeContract,
    snapshot_fingerprint: String,
}
```

`claim_resume_snapshot` 在短写 transaction 中按 operation -> execution/scope -> unit -> plan ->
item -> worker -> tool/chain/receipt 顺序锁行，重新加载同一 fingerprint；只有 exact match 才关闭
prior Turn、插 successor、task=running、执行 V2 Controller reopen 并 commit。

**步骤 4：旧 API 委托新 claim。**

`tasks::claim_exact_resumable_runtime_source` 暂保留，用内部 snapshot + claim 实现兼容现有调用；
新增调用只使用 ticket API。`message_chains::list_exact_resume_bound_chains` 的 projection 移入
snapshot，原函数只做 deprecated facade。

**验证：**

```bash
just space-guard
cd backend && cargo nextest run -p golish-db --test runtime_resume_snapshot --status-level fail
just space-guard
cd backend && cargo nextest run -p golish-db -E 'test(exact_resume_source_claim_allows_only_one_open_turn_contender) | test(exact_resume_claims_running_v2_operation_without_reaper_delay) | test(company_controller_gate_block_reopens_same_worker_chain_until_repair_fuel_is_exhausted)' --status-level fail
```

预期：exit 0；五个新测试和三个现有 atomicity/Controller tests PASS。

**提交：**

```bash
git add backend/crates/golish-db/src/repo/runtime_resume backend/crates/golish-db/src/repo/message_chains.rs backend/crates/golish-db/src/repo/tasks.rs backend/crates/golish-db/tests/runtime_resume_snapshot.rs
git commit -m "feat(resume): claim one repeatable-read resume snapshot"
```

## Task 5：纯 classifier 与 bounded safe-repair fixed point

**文件：**

- 创建 `backend/crates/golish-agent-app/src/ai/resume/mod.rs`
- 创建 `backend/crates/golish-agent-app/src/ai/resume/classify.rs`
- 创建 `backend/crates/golish-agent-app/src/ai/resume/investigation.rs`
- 创建 `backend/crates/golish-agent-app/src/ai/resume/service.rs`
- 修改 `backend/crates/golish-agent-app/src/ai/mod.rs`
- 创建 `backend/crates/golish-agent-app/tests/exact_resume_coordinator.rs`
- 创建 `backend/crates/golish-db/src/repo/runtime_resume/repair.rs`

**步骤 1：写纯分类表驱动 RED tests。**

`ResumeDisposition` 定义为：

```rust
pub enum ResumeDisposition {
    Ready(ResumeDirective),
    SafeRepair(Vec<ResumeRepair>),
    Busy(ResumeBlocker),
    AwaitOperator(ResumeBlocker),
    UnsupportedContract(ResumeContractMismatch),
    Terminal(ResumeTerminal),
    Corrupt(ResumeBlocker),
}
```

表驱动 fixture 至少包含：current dynamic Investigation -> Ready；legacy unversioned ->
Unsupported；live lease -> Busy；active external tool -> AwaitOperator；expired worker/no tool ->
SafeRepair；foreign chain -> Corrupt；completed execution -> Terminal。

运行：

```bash
just space-guard
cd backend && cargo nextest run -p golish-agent-app --test exact_resume_coordinator -E 'test(classify_)' --status-level fail
```

预期：classifier 不存在而 RED。

**步骤 2：实现 explicit stage match。**

```rust
pub fn classify(snapshot: &ResumeSnapshot) -> ResumeDisposition {
    if !snapshot.resume_contract.is_runnable() {
        return ResumeDisposition::UnsupportedContract(ResumeContractMismatch::from(snapshot));
    }
    if snapshot.has_live_lease() {
        return ResumeDisposition::Busy(ResumeBlocker::live_lease(snapshot));
    }
    if snapshot.has_active_external_tool() {
        return ResumeDisposition::AwaitOperator(ResumeBlocker::outcome_unknown(snapshot));
    }
    match snapshot.current_stage.as_str() {
        "investigation" => investigation::classify(snapshot),
        _ => classify_durable_stage_run_v1(snapshot),
    }
}
```

不要通过 trait registry 或模型选择 classifier。

**步骤 3：实现 normalizer。**

```rust
pub const MAX_SAFE_REPAIR_TRANSITIONS: usize = 8;

pub async fn normalize_resume(
    db: &PgPool,
    operation_id: Uuid,
    session_id: Uuid,
) -> anyhow::Result<NormalizedResume> {
    let mut previous: Option<(String, String)> = None;
    for _ in 0..MAX_SAFE_REPAIR_TRANSITIONS {
        let snapshot = select_resume_snapshot(db, operation_id, session_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("resume candidate disappeared"))?;
        match classify(&snapshot) {
            ResumeDisposition::SafeRepair(repairs) => {
                let repair_id = stable_repair_set_id(&repairs);
                if previous.as_ref() == Some(&(snapshot.fingerprint.clone(), repair_id.clone())) {
                    return Ok(NormalizedResume::Deferred(
                        ResumeBlocker::repair_stalled(&snapshot),
                    ));
                }
                apply_repairs(db, &snapshot, &repairs).await?;
                previous = Some((snapshot.fingerprint, repair_id));
            }
            disposition => return Ok(NormalizedResume::Classified(snapshot, disposition)),
        }
    }
    Ok(NormalizedResume::Deferred(ResumeBlocker::repair_budget_exhausted()))
}
```

Repair enum 是 closed allowlist；每个 action 携带 exact preconditions。DB 实现采用已有
`building -> applied` receipt/replay-first 模式，严禁外部 I/O。

**步骤 4：写 fixed-point tests。**

必须断言：三步 repair 后 Ready；同 fingerprint + repair 二次出现为 `RESUME_REPAIR_STALLED`；
第九步不执行；repair CAS drift 为 typed conflict；active tool 从不进入 repair executor。

**验证：**

```bash
just space-guard
cd backend && cargo nextest run -p golish-agent-app --test exact_resume_coordinator --status-level fail
just space-guard
cd backend && cargo nextest run -p golish-db -E 'test(stage_team_gate_repair_advances_epoch_and_blocks_only_fresh_aggregator) | test(expired_worker_with_active_tool_requires_recovery_and_is_not_reclaimable)' --status-level fail
```

预期：exit 0，classification matrix 与 fixed-point tests 全 PASS。

**提交：**

```bash
git add backend/crates/golish-agent-app/src/ai/resume backend/crates/golish-agent-app/src/ai/mod.rs backend/crates/golish-agent-app/tests/exact_resume_coordinator.rs backend/crates/golish-db/src/repo/runtime_resume/repair.rs
git commit -m "feat(resume): normalize safe recovery to a fixed point"
```

## Task 6：让 GUI、Candidate Review、CLI 只消费 claim ticket

**文件：**

- 修改 `backend/crates/golish-agent-app/src/ai/commands/core/chat.rs`
- 修改 `backend/crates/golish-agent-app/src/ai/commands/core/operation_resume.rs`
- 修改 `backend/crates/golish-agent-app/src/ai/commands/attack.rs`
- 修改 `backend/crates/golish-agent-kit/src/task_orchestrator/orchestrator.rs`
- 修改 `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`
- 修改 `backend/crates/golish-agent-bridge/src/agent_bridge/config.rs`
- 修改 `backend/crates/golish-agent-bridge/src/agent_bridge/prepare.rs`
- 修改 `backend/crates/golish-agent-bridge/src/agent_bridge/mod.rs`
- 修改 `backend/crates/golish/src/stage_run/mod.rs`
- 修改 `backend/crates/golish/src/stage_run/runtime_v2.rs`
- 修改 `backend/crates/golish-agent-app/tests/exact_resume_coordinator.rs`

**步骤 1：先写 adapter conformance RED。**

Fake coordinator 记录调用，GUI、Candidate 和 CLI 各断言一次：

```rust
assert_eq!(calls.len(), 1);
assert_eq!(calls[0].operation_id, expected_operation_id);
assert_eq!(calls[0].session_id, expected_session_id);
assert_eq!(calls[0].trigger_input, expected_trigger);
assert_eq!(calls[0].claim_count, 1);
```

再写 `resume_lookup_db_error_does_not_start_fresh_operation`：fake 返回 Err，断言 fresh runner
调用次数为 0。`NoCandidate` 才断言 fresh runner=1。

**步骤 2：coordinator 返回 closed preparation outcome。**

```rust
pub enum PrepareResumeOutcome {
    NoCandidate,
    Claimed(ClaimedExactResume),
    Deferred(ResumeBlocker),
    Terminal(ResumeTerminal),
}
```

DB error 通过 `Result::Err` 向上传播，不能转换成 `NoCandidate`。

**步骤 3：orchestrator 消费 ticket。**

```rust
pub async fn resume_claimed(
    &mut self,
    claimed: ClaimedExactResume,
    user_message: &str,
    executor: &dyn AgentExecutor,
) -> Result<String> {
    let task_id = claimed.operation_id();
    self.resume_runtime_memory_source = Some(claimed.source().into());
    self.run_claimed_executor(task_id, claimed, user_message, executor).await
}
```

生产调用不再暴露 `set_resume_runtime_memory_source`、`set_resume_task_preclaimed` 或 generic
unfenced `resume`。Ticket 不能 Clone，进入 `resume_claimed` 后被 move，重复使用编译失败。

**步骤 4：移除三份 mutable pin 排列。**

GUI/Candidate/CLI 都改成：authorize adapter-specific scope/barrier -> `prepare_and_claim` ->
`resume_claimed(ticket)`。Bridge source 从 request execution context 派生，不再单独 setter。

CLI 的 advisory lock、workspace/transcript/provider validation 留在 CLI；
`runtime_v2::load_relational_resume_authority` 不再决定 canonical resume disposition。

**步骤 5：替换 include_str 顺序测试。**

删除只检查源码字符串顺序的测试，保留/新增 fake service 行为测试和 DB atomicity tests。

**验证：**

```bash
just space-guard
cd backend && cargo nextest run -p golish-agent-app --test exact_resume_coordinator --status-level fail
just space-guard
cd backend && cargo nextest run -p golish-agent-kit -E 'test(preferred_resume_obeys_the_selected_whole_record_source) | test(resume_claimed_consumes_ticket_once)' --status-level fail
just space-guard
cd backend && cargo nextest run -p golish -E 'test(stage_run_resume_uses_shared_operation_turn_claim) | test(durable_resume_claim_runs_immediately_before_orchestrator_resume)' --status-level fail
```

预期：exit 0；DB error/fresh、三个 adapter、ticket/source/Turn 全部 PASS。

**提交：**

```bash
git add backend/crates/golish-agent-app/src/ai/commands/core/chat.rs backend/crates/golish-agent-app/src/ai/commands/core/operation_resume.rs backend/crates/golish-agent-app/src/ai/commands/attack.rs backend/crates/golish-agent-app/tests/exact_resume_coordinator.rs backend/crates/golish-agent-kit/src/task_orchestrator backend/crates/golish-agent-bridge/src/agent_bridge backend/crates/golish/src/stage_run
git commit -m "refactor(resume): route every adapter through claimed resume"
```

## Task 7：关闭 tracked tool / WorkerRun 的两个 crash window

**文件：**

- 创建 `backend/crates/golish-db/src/repo/runtime_resume/tool_fence.rs`
- 创建 `backend/crates/golish-db/tests/runtime_tool_lifecycle.rs`
- 修改 `backend/crates/golish-db/src/repo/runtime_resume/mod.rs`
- 修改 `backend/crates/golish-db/src/repo/tool_calls.rs`
- 修改 `backend/crates/golish-db/src/repo/runtime_memory_tx.rs`
- 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/worker_tool_lifecycle.rs`
- 修改 `backend/crates/golish-agent-kit/src/db_traits/runtime_memory.rs`
- 修改 `backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs`

**步骤 1：写 atomicity RED tests。**

四个测试必须断言：begin 的 tool row + worker tuple 同 commit/rollback；finish 的 terminal tool +
worker clear 同 commit/rollback；crash-after-begin 为 outcome unknown；duplicate begin/finish exact
replay、mismatched attempt conflict。

**步骤 2：实现两个短 transaction API。**

```rust
pub async fn begin_tracked_worker_tool(
    pool: &PgPool,
    input: BeginTrackedWorkerTool,
) -> RuntimeMemoryStoreResult<TrackedToolFence>;

pub async fn finish_tracked_worker_tool(
    pool: &PgPool,
    input: FinishTrackedWorkerTool,
) -> RuntimeMemoryStoreResult<TrackedToolOutcome>;
```

Begin 锁 worker，校验 operation/unit/plan/item/attempt/lease 后插 tool row 并 CAS active tuple；commit
后 runtime 才调用外部工具。Finish 锁同一 worker/tool，写 result/status 后清 exact tuple，一次
commit。事务中没有 HTTP、shell、provider 或 MQ。

**步骤 3：runtime 只调用 atomic API。**

删除 `worker_tool_lifecycle.rs` 中“先 tool start commit，再 worker begin commit”和“先 worker
clear，再 tool terminalize”的顺序；所有错误映射为 typed fence conflict/outcome unknown。
`RuntimeMemoryRepository` 新增 `begin_tracked_worker_tool` / `finish_tracked_worker_tool`，app DB
bridge 只做 DTO 转换和 repository 委托；测试 fake 必须原子记录完整 outcome，不能重新拆成两次
trait 调用。

**验证：**

```bash
just space-guard
cd backend && cargo nextest run -p golish-db --test runtime_tool_lifecycle --status-level fail
just space-guard
cd backend && cargo nextest run -p golish-db -E 'test(expired_worker_with_active_tool_requires_recovery_and_is_not_reclaimable)' --status-level fail
just space-guard
cd backend && cargo nextest run -p golish-agent-runtime -E 'test(worker_tool_)' --status-level fail
```

预期：exit 0；atomic/replay/outcome-unknown tests PASS。

**提交：**

```bash
git add backend/crates/golish-db/src/repo/runtime_resume/tool_fence.rs backend/crates/golish-db/src/repo/runtime_resume/mod.rs backend/crates/golish-db/src/repo/tool_calls.rs backend/crates/golish-db/src/repo/runtime_memory_tx.rs backend/crates/golish-db/tests/runtime_tool_lifecycle.rs backend/crates/golish-agent-runtime/src/agentic_loop/worker_tool_lifecycle.rs backend/crates/golish-agent-kit/src/db_traits/runtime_memory.rs backend/crates/golish-agent-app/src/ai/db_bridge/runtime_memory.rs
git commit -m "fix(resume): make worker tool fences atomic"
```

## Task 8：typed stage_run control 与非 terminal resume finalization

**文件：**

- 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/context.rs`
- 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_dispatch.rs`
- 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`
- 修改 `backend/crates/golish-agent-kit/src/task_orchestrator/orchestrator.rs`

**步骤 1：写 runtime-control RED tests。**

测试固定以下语义：

```rust
assert_eq!(runtime_recovered.retry, RetryDisposition::NewRequest);
assert_eq!(operator_recovery.retry, RetryDisposition::AfterOperatorResolution);
assert_eq!(fuel_exhausted.retry, RetryDisposition::Exhausted);
assert_ne!(runtime_recovered.retry, fuel_exhausted.retry);
assert!(StageRunRuntimeControl::from_value(&unknown_reason).is_err());
```

再加 `deterministic_pre_provider_blocker_leaves_task_waiting`：结果 `status=waiting`、`result=NULL`、
provider dispatch count=0。第二次同 snapshot 得同 fingerprint，dispatch 仍为 0。

**步骤 2：一个 serializer、一个 parser。**

所有 `stage_run` halt 都通过 `control.rs` 构造 `StageRunRuntimeControl`，JSON 只作为 UI/wire
projection。`tool_dispatch.rs` 执行：

```rust
let control = value
    .get("runtime_control")
    .map(StageRunRuntimeControl::from_value)
    .transpose()?;
```

删除 reason + passed + scheduler + gaps + retry_budget_exhausted 的相关字段推断。

**步骤 3：reentry guard 保存 token。**

```rust
pub struct HaltToken {
    pub stage: StageKind,
    pub blocker_fingerprint: String,
    pub reason: StageRunHaltReason,
}
```

Guard 从 `HashSet<StageKind>` 改为 `HashMap<StageKind, HaltToken>`。同 request 同 token 不再调用
stage_run；不同 token 仍需 classifier 明确允许，guard 自身不成为 durable authority。

**步骤 4：区分 deferred 与 terminal。**

`TaskOrchestrator` 对 typed `ResumeExecutionHalt::Deferred(control)` 写 task waiting/result NULL；
只有 `ResumeExecutionHalt::Fatal` 或明确 terminal policy 调 `fail_task_if_active`。Repository
infrastructure、human hold、outcome unknown、runtime recovered 和 repair stalled 都不是 fatal。

**验证：**

```bash
just space-guard
cd backend && cargo nextest run -p golish-agent-runtime -E 'test(company_controller_recovery_gap_requires_operator_and_stops_request_reentry) | test(company_controller_ordinary_gap_stops_the_current_request) | test(company_controller_final_seal_gap_forbids_gate_repair_and_rescan) | test(company_controller_runtime_recovery_preserves_facts_and_stops_old_execution) | test(company_controller_missing_submission_reports_final_submitter_resume_not_gate_block) | test(company_controller_final_gaps_exhaust_only_the_current_request_guard) | test(runtime_control_is_closed_to_stage_run_and_known_reasons)' --status-level fail
just space-guard
cd backend && cargo nextest run -p golish-agent-kit -E 'test(deterministic_pre_provider_blocker_leaves_task_waiting)' --status-level fail
```

预期：exit 0；typed control 与 waiting finalization 全 PASS。

**提交：**

```bash
git add backend/crates/golish-agent-runtime/src/agentic_loop/context.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_dispatch.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs backend/crates/golish-agent-kit/src/task_orchestrator/orchestrator.rs
git commit -m "fix(resume): preserve typed blockers without terminal failure"
```

## Task 9：行为稳定后按 seam 拆大文件

**文件：**

- 创建 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run/{mod.rs,snapshot.rs,classify.rs,repair.rs,dispatch.rs,control.rs}`
- 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs`
- 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs`
- 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_team_scheduler.rs`
- 创建 `backend/crates/golish-db/src/repo/runtime_resume/reaper.rs`
- 修改 `backend/crates/golish-db/src/repo/runtime_memory_tx.rs`
- 修改 `backend/crates/golish-db/src/repo/tasks.rs`
- 修改 `backend/crates/golish/src/stage_run/runtime_v2.rs`

**步骤 1：记录提取前 focused baseline。**

运行本 Task 验证段全部命令并把测试名/exit code 记入 `agent-progress.md`。任何 baseline failure
先归因，不在 extraction 中顺手修业务。

**步骤 2：机械迁移，commit 内不改 SQL/branch condition。**

- `stage_run/snapshot.rs`：只搬 DB/runtime view 组装。
- `stage_run/classify.rs`：只搬纯 action decision。
- `stage_run/repair.rs`：只搬 fenced deterministic mutation adapter。
- `stage_run/dispatch.rs`：只搬 model/provider/semaphore 调用。
- `stage_run/control.rs`：只搬 typed projection。
- `stage_run_call.rs` 留 facade，调用固定顺序 `load -> classify -> repair/reload | dispatch once | halt`。

`runtime_memory_tx.rs` 只迁出已被 new facade 覆盖的 resume/reaper functions；旧 public symbol 先
`pub use`，调用方逐个切换后再删 facade。`runtime_v2.rs` 只留 CLI display/workspace diagnostic，
不再产生 `ResumeDisposition`。

**步骤 3：用文件职责测试替代源码顺序测试。**

删除 `include_str!` 查字符串位置的 tests；保留行为 tests。加一个 compile-time module visibility
test，确保 `dispatch` 是唯一能访问 model/provider 的 stage_run child module。

**验证：**

```bash
just space-guard
cd backend && cargo nextest run -p golish -E 'test(stage_team_resume_selects_unique_controller_and_accepts_dynamic_children) | test(stage_team_resume_rejects_duplicate_leader_or_foreign_child) | test(resumability_distinguishes_scoping_specialist_and_root_only_units) | test(target_intel_finalizer_recovery_admits_only_the_exact_terminal_witness) | test(malformed_or_cross_operation_runtime_identity_fails_closed)' --status-level fail
just space-guard
cd backend && cargo nextest run -p golish-agent-runtime -E 'test(company_controller_turn_accepts_only_host_barrier_results) | test(company_controller_turn_accepts_matching_durable_chain_marker) | test(company_controller_turn_rejects_untrusted_durable_chain_marker) | test(company_controller_waiting_action_drains_unclaimed_queued_child) | test(company_controller_closed_plan_resumes_through_final_submitter_claim)' --status-level fail
just space-guard
cd backend && cargo nextest run -p golish-db --test runtime_resume_snapshot --test runtime_resume_reaper --test runtime_tool_lifecycle --status-level fail
```

预期：提取前后同一组 tests exit 0，行为与 fingerprint 不变。

**提交：**

```bash
git add backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct backend/crates/golish-db/src/repo/runtime_resume backend/crates/golish-db/src/repo/runtime_memory_tx.rs backend/crates/golish-db/src/repo/tasks.rs backend/crates/golish/src/stage_run/runtime_v2.rs
git commit -m "refactor(resume): separate snapshot repair and dispatch seams"
```

## Task 10：协议 gate 生效后退休 Investigation 旧执行兼容

> 执行本 Task 前先取得用户对删除旧执行代码的明确确认。Global Legacy/Dual storage-source
> retirement 不在本 Task。

**文件：**

- 修改 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run/`
- 修改 `backend/crates/golish-db/src/repo/runtime_resume/authority.rs`
- 修改 `backend/crates/golish-db/src/repo/runtime_resume/snapshot.rs`
- 修改 `backend/crates/golish-db/src/repo/runtime_memory_tx.rs`
- 修改 `backend/crates/golish/src/stage_run/runtime_v2.rs`
- 修改相关 focused tests；不删除旧 migrations/receipt rows/history readers

**步骤 1：先跑只读 census。**

在目标 DB 执行并保存结果：

```sql
SELECT run.resume_contract, run.stage_kind, run.status, COUNT(*)
  FROM stage_runs run
 GROUP BY run.resume_contract, run.stage_kind, run.status
 ORDER BY run.resume_contract, run.stage_kind, run.status;

SELECT operation.runtime_memory_contract, task.status, COUNT(*)
  FROM operation_state operation
  JOIN tasks task ON task.id = operation.operation_id
 WHERE task.status IN ('created','running','waiting') AND task.result IS NULL
 GROUP BY operation.runtime_memory_contract, task.status
 ORDER BY operation.runtime_memory_contract, task.status;
```

只有 new resume gate 已部署、旧 active stage 会 typed reject、当前 contract fixture GREEN 后才继续。

**步骤 2：写 retirement RED。**

```rust
assert_matches!(
    prepare_legacy_unversioned_investigation().await,
    PrepareResumeOutcome::Deferred(blocker)
        if blocker.code == "UNSUPPORTED_FROZEN_CONTRACT"
);
assert_eq!(provider_dispatch_count(), 0);
assert_eq!(successor_turn_count(), 0);
assert!(historical_fixed_receipt_is_queryable());
```

**步骤 3：按 caller graph 删除 runnable compatibility。**

删除 fixed-roster/old-plan 作为 runnable authority 的 selector、scheduler、cutover fallback 和
generic error recovery；保留历史查询、append-only receipt、migration、FK RESTRICT 和所有
scope/identity/lease/tool/chain fence。删除一个 production caller 后立刻运行相关 focused test，
不做跨域“清理”。

**步骤 4：明确保留 storage source compatibility。**

`ResumeRecordSource::{Legacy,LegacyFallback}` 仍只在 isolated `source_policy` 中存在；V2-only
ticket 不暴露 fallback。Global storage retirement 需要独立设计、发布窗口、全库 census 与用户
授权，不能在本 commit 顺带完成。

**验证：**

```bash
just space-guard
cd backend && cargo nextest run -p golish-agent-app --test exact_resume_coordinator -E 'test(legacy_unversioned_investigation_is_unsupported_without_dispatch)' --status-level fail
just space-guard
cd backend && cargo nextest run -p golish-db --test runtime_resume_protocol --test runtime_resume_snapshot --status-level fail
just space-guard
cd backend && cargo nextest run -p golish-agent-runtime -E 'test(investigation_asset_) | test(runtime_control_is_closed_to_stage_run_and_known_reasons)' --status-level fail
```

预期：exit 0；旧 active protocol typed reject，current protocol resumes，history remains readable。

**提交：**

```bash
git add backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run backend/crates/golish-db/src/repo/runtime_resume backend/crates/golish-db/src/repo/runtime_memory_tx.rs backend/crates/golish/src/stage_run/runtime_v2.rs backend/crates/golish-agent-app/tests/exact_resume_coordinator.rs backend/crates/golish-db/tests/runtime_resume_protocol.rs backend/crates/golish-db/tests/runtime_resume_snapshot.rs
git commit -m "refactor(resume): retire unsupported investigation runtimes"
```

## Task 11：模块卡、定向门禁与 controlled acceptance

**文件：**

- 修改 `docs/modules/backend/golish-core.md`
- 修改 `docs/modules/backend/golish-db.md`
- 修改 `docs/modules/backend/golish-db/repo.md`
- 修改 `docs/modules/backend/golish-agent-app/ai.md`
- 修改 `docs/modules/backend/golish-agent-kit/task_orchestrator.md`
- 修改 `docs/modules/backend/golish-agent-runtime.md`
- 修改 `docs/modules/backend/golish-agent-runtime/agentic_loop.md`
- 修改 `docs/modules/backend/golish/stage_run.md`
- 修改 `docs/modules/INDEX.md`
- 修改 `feature_list.json`
- 修改 `agent-progress.md`

**步骤 1：更新 module system-of-record。**

每张卡必须写清：唯一 public resume facade、snapshot/claim transaction、ticket consumer、typed
halt、tool fence 与测试入口。INDEX 状态列同步为当前实现，不复制设计全文。

**步骤 2：运行 scoped final gates。**

```bash
just space-guard
cd backend && cargo nextest run -p golish-core -p golish-db -p golish-agent-app -p golish-agent-kit -p golish-agent-runtime -p golish -E 'test(resume_) | test(exact_resume_) | test(runtime_control_) | test(worker_tool_)' --status-level fail
just space-guard
cd backend && cargo clippy -p golish-core -p golish-db -p golish-agent-app -p golish-agent-kit -p golish-agent-runtime -p golish --tests --no-deps -- -D warnings
cargo fmt --manifest-path backend/Cargo.toml --all -- --check
jq empty feature_list.json
jq -e '([.features[] | select(.status == "in_progress")] | length) <= 1' feature_list.json
git diff --check
```

预期：全部 exit 0；Clippy 零 warning；JSON 有效；最多一个 `in_progress`；diff 无 whitespace error。

**步骤 3：controlled no-provider acceptance。**

用 embedded PG fixture 验证以下时间线，provider fake 的 dispatch count 必须精确：

```text
DB lookup error -> Err, fresh=0, provider=0
unsupported contract -> Deferred, Turn delta=0, provider=0
expired worker -> SafeRepair -> Ready -> one ticket -> Turn delta=1, provider=1
same blocker second continue -> same fingerprint, Turn delta=0, provider=0
active tool outcome unknown -> AwaitOperator, tool replay=0, provider=0
```

**步骤 4：fresh current-contract entity resume。**

只有用户另行授权真实 provider/target I/O 时，才从 final-sealed predecessor fork 一个新的
Investigation stage，验证同一 Asset Primary chain、无重复 tool execution 和 stage closure。未授权
时以 controlled acceptance 为本计划完成证据，并在 feature evidence 明确真实实体门禁未运行。

**步骤 5：更新状态。**

把执行命令、exit code、关键 fingerprint/Turn/provider/tool 计数写入 `agent-progress.md`。所有
验收通过才把 feature 设为 `passing`；缺 schema/deletion 授权时保持 `not_started|blocked`，代码
半成品保持 `in_progress`。不以未授权的全仓门禁为 blocker。

**提交：**

```bash
git add docs/modules feature_list.json agent-progress.md
git commit -m "docs(resume): record normalized resume architecture"
```

## 计划自检

- **规格覆盖度：** Task 1 定义稳定 vocabulary；Task 2 区分 behavior protocol；Task 3 统一
  recoverability；Task 4 保证 single snapshot + atomic ticket；Task 5 完成 classify/repair fixed
  point；Task 6 统一三个入口；Task 7 关闭 tool crash window；Task 8 解决重复 blocker 与误标
  failed；Task 9 在行为锁定后解耦大文件；Task 10 安全退休 runtime compatibility；Task 11
  完成定向证据和模块交接。
- **权限覆盖度：** migration 与删除分别有执行前确认；外部 provider/target I/O 另行授权；没有
  删除历史 migration/evidence。
- **类型一致性：** 全计划统一使用 `StageResumeContract`、`ResumeSnapshot`、
  `ResumeDisposition`、`ClaimedExactResume`、`StageRunRuntimeControl`、`ResumeBlocker` 与
  `RetryDisposition`；只有 `Ready` 可生成 ticket，只有 ticket 可进入 production resume。
- **边界一致性：** DB transaction 内只做 authority read/CAS/receipt；provider/tool dispatch 永远
  在 commit 后；request guard 是 circuit breaker，不是 durable truth。
- **占位符检查：** 所有任务均给出精确文件、API/SQL contract、验证命令、预期结果和独立提交。
