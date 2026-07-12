# Cleanup Obligation Ledger 与可验证收尾实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 让每个可能产生副作用的 post-exploit action 同事务创建 cleanup obligation，并通过独立 Attempt、absence evidence 和 residual risk 完成可恢复、可审计的 Cleanup stage。

**架构：** Cleanup obligation 是安全债务账本，不能被普通 feature flag 绕过。所有副作用先由 P6 在同一短事务创建 `prepared` action + exact cleanup obligation，并调用 Memory Core enqueue API；该 API 追加 immutable `knowledge_outbox_events` 并建立 required `knowledge_projection_deliveries`，事务提交后才执行外部动作。Cleanup 使用独立 Attempt 清理，再由不同 verifier/run 产生独立 absence evidence；投影消费者只能更新自己的 delivery，本域不直接写 event/delivery/status 表。`cleaned` 不是完成，只有 `verified_absent` 才是无残留终态；blocked/可信用户 waiver 必须进入报告 residual。Gate 除查询 obligation 终态外，还必须 LEFT JOIN 查询任何“已有副作用 action 但缺 obligation”的 invariant violation。关闭 post-exploit/cleanup UI 或执行 feature flag 只能停止新增动作，不能停止 open obligation 的 claim、reconcile、absence verify 和恢复。

**技术栈：** Rust 2021、sqlx/PostgreSQL、harness capability/Gate、Tauri/ts-rs、React/Vitest。

**依赖：** P3 Memory Fabric Core + P6 Post-exploit Domain；schema/真实 cleanup action/waiver IPC 实施前取得用户确认。

---

## 1. 文件结构

### 新建

- `backend/crates/golish-cleanup-domain/{Cargo.toml,src/lib.rs}`
- `backend/crates/golish-cleanup-domain/src/{obligation.rs,attempt.rs,residual.rs,events.rs}`
- `backend/crates/golish-cleanup-app/{Cargo.toml,src/lib.rs}`
- `backend/crates/golish-cleanup-app/src/{ports.rs,service.rs,capabilities.rs,absence_verifier.rs,reconcile.rs,recovery.rs}`
- `backend/crates/golish-db/migrations/20260712000006_cleanup_obligation_ledger.sql`
- `backend/crates/golish-db/src/repo/{cleanup_obligations.rs,cleanup_attempts.rs,cleanup_absence_checks.rs,cleanup_waivers.rs}`
- `resources/harness/stages/cleanup/methodology.md`
- `backend/crates/golish-agent-app/src/ai/commands/cleanup.rs`
- `backend/crates/golish/src/commands_facade/cleanup.rs`
- `frontend/lib/api/cleanup.ts`
- `frontend/components/Engagement/{CleanupObligationList.tsx,CleanupObligationList.test.tsx}`

### 修改

- `backend/Cargo.toml`
- `golish-post-exploit-app` action transaction integration。
- `golish-db/src/repo/mod.rs`
- harness stage capability/gate/context/spec。
- `backend/crates/golish-agent-bridge/src/agent_bridge/config.rs`（开关只控制新增动作，不控制 recovery）。
- `backend/crates/golish/src/app/bootstrap.rs`（启动时按 DB open work 无条件拉起 cleanup recovery supervisor）。
- app commands facade/registry、ts-rs export。
- `backend/crates/golish-recon-app/src/organizations/artifact_cleanup.rs`
- 模块卡/INDEX。

---

## Task 0：先建立模块卡和不可关闭的恢复边界

写任何 crate、migration 或 IPC 前，先创建：

- `docs/modules/backend/golish-cleanup-domain.md`
- `docs/modules/backend/golish-cleanup-app.md`

并更新 post-exploit app、DB、harness、organization cleanup、frontend 对应模块卡及 `docs/modules/INDEX.md`。模块卡必须先写明：prepared action/obligation 原子边界、独立 absence verifier、trusted waiver、org delete blocker、feature flag 不能关闭 recovery，以及测试入口。

验证并提交文档骨架：

```bash
rg -n "cleanup obligation|recovery|verified_absent" docs/modules/backend docs/modules/INDEX.md
just precommit
git add docs/modules/backend/golish-cleanup-domain.md docs/modules/backend/golish-cleanup-app.md docs/modules/backend/golish-post-exploit-app.md docs/modules/backend/golish-db.md docs/modules/backend/golish-agent-kit.md docs/modules/backend/golish-agent-bridge/agent_bridge.md docs/modules/backend/golish/app.md docs/modules/backend/golish-recon-app/organizations.md docs/modules/frontend/components.md docs/modules/INDEX.md
git commit -m "docs(cleanup): define safety module boundaries before implementation"
```

---

## Task 1：定义 obligation/attempt/residual 领域状态机

**文件：** cleanup-domain。

### 步骤 1：写 RED

```rust
#[test]
fn side_effect_integrity_requires_exact_durable_obligation() {
    let action = side_effect_action(action_id(1), resource("remote-user:alice"));
    let missing = SideEffectIntegrityFact::from_action(&action, None);
    assert_eq!(validate_side_effect_integrity(&missing), Err(CleanupError::MissingObligation));

    let wrong_resource = obligation(obligation_id(2), action.action_id, resource("remote-user:bob"));
    assert_eq!(validate_side_effect_integrity(&SideEffectIntegrityFact::from_action(&action, Some(wrong_resource))),
               Err(CleanupError::ResourceIdentityMismatch));

    let exact = obligation(obligation_id(3), action.action_id, action.affected_resource_snapshot.clone());
    assert!(validate_side_effect_integrity(&SideEffectIntegrityFact::from_action(&action, Some(exact))).is_ok());
}

#[test]
fn cleaned_needs_independent_passed_absence_check() {
    let cleaned = obligation_in(CleanupObligationStatus::Cleaned);
    assert_eq!(terminalize_obligation(&cleaned, None), Err(CleanupError::MissingAbsenceCheck));

    let reused = absence_check()
        .with_verifier_run(cleaned.cleanup_worker_run_id())
        .with_evidence(cleaned.cleanup_evidence_refs().to_vec());
    assert_eq!(terminalize_obligation(&cleaned, Some(reused)), Err(CleanupError::AbsenceCheckNotIndependent));

    let independent = absence_check()
        .with_different_verifier_run(worker_run_id(9))
        .with_evidence(vec![absence_evidence(71)]);
    assert_eq!(terminalize_obligation(&cleaned, Some(independent)).unwrap(), CleanupObligationStatus::VerifiedAbsent);
}

#[test]
fn residual_and_terminal_transition_matrix_is_fail_closed() {
    for status in [CleanupObligationStatus::Blocked, CleanupObligationStatus::WaivedByUser] {
        let obligation = obligation_in(status);
        let residual = residual_for_terminal(&obligation).expect("blocked/waived 必须产生 residual");
        assert_eq!(residual.obligation_id, obligation.obligation_id);
        assert!(!residual.reason.is_empty());
    }

    for terminal in [CleanupObligationStatus::VerifiedAbsent, CleanupObligationStatus::Blocked, CleanupObligationStatus::WaivedByUser] {
        assert_eq!(transition_obligation(terminal, CleanupEvent::Start), Err(CleanupError::TerminalState));
    }
}

#[test]
fn waiver_requires_trusted_actor_reason_and_independent_evidence() {
    let request = WaiverRequest { reason: "target owner retained test account".into(), evidence_refs: vec![72] };
    assert_eq!(validate_waiver(&request, None), Err(CleanupError::MissingTrustedActor));

    let model_actor = TrustedDecisionContext::model("agent-1");
    assert_eq!(validate_waiver(&request, Some(&model_actor)), Err(CleanupError::UntrustedWaiverActor));

    let user_actor = TrustedDecisionContext::authenticated_user("user-7", "session-3");
    assert!(validate_waiver(&request, Some(&user_actor)).is_ok());
}
```

### 步骤 2：定义类型

```rust
pub enum CleanupObligationStatus {
    Pending,
    Running,
    Cleaned,
    VerifiedAbsent,
    Blocked,
    WaivedByUser,
}

pub enum CleanupAttemptStatus {
    Queued,
    Leased,
    Running,
    Submitted,
    Cleaned,
    Blocked,
    RetryableFailed,
    Abandoned,
}

pub struct CleanupObligation {
    pub obligation_id: Uuid,
    pub operation_id: Uuid,
    pub organization_id_at_time: Uuid,
    pub source_action_id: Uuid,
    pub affected_resource_snapshot: Value,
    pub resource_identity_hash: String,
    pub cleanup_strategy: Value,
    pub proof_requirements: Vec<AbsenceProofRequirement>,
    pub deadline: DateTime<Utc>,
    pub status: CleanupObligationStatus,
    pub scope_snapshot_hash: String,
}

pub struct CleanupAbsenceCheck {
    pub absence_check_id: Uuid,
    pub obligation_id: Uuid,
    pub cleanup_attempt_id: Uuid,
    pub verifier_worker_run_id: Uuid,
    pub resource_identity_hash: String,
    pub disposition: AbsenceDisposition,
    pub evidence_refs: Vec<i64>,
    pub checked_at: DateTime<Utc>,
}

pub struct CleanupWaiverDecision {
    pub waiver_id: Uuid,
    pub obligation_id: Uuid,
    pub decision_version: i64,
    pub trusted_user_id: String,
    pub trusted_session_id: String,
    pub reason: String,
    pub evidence_refs: Vec<i64>,
    pub decided_at: DateTime<Utc>,
}
```

### 步骤 3：实现纯函数

```rust
pub fn transition_obligation(from: CleanupObligationStatus, event: CleanupEvent) -> Result<CleanupObligationStatus, CleanupError>;
pub fn validate_side_effect_integrity(input: &SideEffectIntegrityFact) -> Result<(), CleanupError>;
pub fn validate_waiver(input: &WaiverRequest, actor: Option<&TrustedDecisionContext>) -> Result<(), CleanupError>;
pub fn validate_absence_check(obligation: &CleanupObligation, check: &CleanupAbsenceCheck) -> Result<(), CleanupError>;
pub fn residual_for_terminal(obligation: &CleanupObligation) -> Option<ResidualRisk>;
```

### 步骤 4：GREEN 与提交

```bash
cd backend && cargo nextest run -p golish-cleanup-domain --status-level fail --no-tests=fail
just precommit
```

```bash
git add backend/Cargo.toml backend/crates/golish-cleanup-domain/Cargo.toml backend/crates/golish-cleanup-domain/src/lib.rs backend/crates/golish-cleanup-domain/src/obligation.rs backend/crates/golish-cleanup-domain/src/attempt.rs backend/crates/golish-cleanup-domain/src/residual.rs backend/crates/golish-cleanup-domain/src/events.rs
git commit -m "feat(cleanup): define obligation and residual state machine"
```

---

## Task 2：新增 cleanup schema/repo

**文件：** migration/repo。

### 步骤 1：写 RED

```rust
#[sqlx::test]
async fn prepared_side_effect_and_obligation_commit_or_rollback_together(pool: PgPool) {
    let command = prepare_side_effect("idem-cleanup-1", org_id(1), resource("remote-user:alice"));
    inject_obligation_insert_failure(&pool).await;
    assert_db_code(prepare_action_with_obligation(&pool, command).await, "CLEANUP_OBLIGATION_PREPARE_FAILED");
    assert_eq!(count_post_exploit_actions(&pool).await, 0);
    assert_eq!(count_cleanup_obligations(&pool).await, 0);
}

#[sqlx::test]
async fn one_live_attempt_includes_queued_and_claim_is_idempotent(pool: PgPool) {
    let obligation = seed_pending_obligation(&pool, org_id(1)).await;
    let queued = enqueue_cleanup_attempt(&pool, obligation.id, 1).await.unwrap();
    assert_unique_violation(enqueue_cleanup_attempt(&pool, obligation.id, 2).await);

    let claimed = claim_next_obligation(&pool, cleanup_query(org_id(1))).await.unwrap().unwrap();
    assert_eq!(claimed.cleanup_attempt_id, queued.cleanup_attempt_id);
    assert_eq!(count_live_attempts(&pool, obligation.id).await, 1);
}

#[sqlx::test]
async fn missing_obligation_query_finds_every_side_effect_action(pool: PgPool) {
    let good = seed_prepared_action_with_obligation(&pool, org_id(1)).await;
    let bad = seed_legacy_side_effect_without_obligation(&pool, org_id(1)).await;
    let violations = list_side_effects_missing_obligations(&pool, operation_id(1), org_id(1)).await.unwrap();
    assert_eq!(ids(&violations), [bad.action_id]);
    assert!(!ids(&violations).contains(&good.action_id));
}

#[sqlx::test]
async fn absence_and_waiver_provenance_are_append_only_and_org_scoped(pool: PgPool) {
    let obligation = seed_cleaned_obligation(&pool, org_id(1)).await;
    let wrong_org = absence_check_write(obligation.id, org_id(2), worker_run_id(9), vec![81]);
    assert_db_code(insert_absence_check(&pool, wrong_org).await, "CLEANUP_ORG_SCOPE_MISMATCH");

    let waiver = trusted_waiver_write(obligation.id, "user-7", "session-3", "owner accepted residual", vec![82]);
    let row = insert_waiver_decision(&pool, waiver).await.unwrap();
    assert_eq!(row.trusted_user_id, "user-7");
    assert_unique_violation(insert_waiver_decision(&pool, replay_waiver(row)).await);
}
```

### 步骤 2：创建表

Retention matrix：cleanup 历史行保存 immutable `organization_id_at_time`，不 FK live `organizations`；affected resource 使用 canonical snapshot/hash，不依赖可删除的 live target/resource row。新表之间用 operation + organization-at-time + scope hash 的 composite constraint，外部 source action 由 repo 锁行交叉验证。这样 nonterminal obligation 能在删除前阻断，但 terminal cleanup 历史不会反过来要求保留 live organization row。

```sql
CREATE TABLE cleanup_obligations (
    obligation_id UUID PRIMARY KEY,
    operation_id UUID NOT NULL REFERENCES operation_state(operation_id),
    organization_id_at_time UUID NOT NULL,
    source_action_id UUID NOT NULL UNIQUE,
    affected_resource_snapshot JSONB NOT NULL,
    resource_identity_hash TEXT NOT NULL,
    cleanup_strategy JSONB NOT NULL,
    proof_requirements JSONB NOT NULL,
    deadline TIMESTAMPTZ NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending','running','cleaned','verified_absent','blocked','waived_by_user')),
    scope_snapshot_hash TEXT NOT NULL,
    residual_risk JSONB,
    row_version BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    terminal_at TIMESTAMPTZ,
    UNIQUE(obligation_id, operation_id, organization_id_at_time, scope_snapshot_hash),
    FOREIGN KEY(source_action_id, operation_id, organization_id_at_time, scope_snapshot_hash)
        REFERENCES post_exploit_actions(action_id, operation_id, organization_id_at_time, scope_snapshot_hash)
        ON DELETE RESTRICT
);

CREATE TABLE cleanup_attempts (
    cleanup_attempt_id UUID PRIMARY KEY,
    obligation_id UUID NOT NULL REFERENCES cleanup_obligations(obligation_id) ON DELETE RESTRICT,
    ordinal INTEGER NOT NULL CHECK (ordinal > 0),
    status TEXT NOT NULL CHECK (status IN ('queued','leased','running','submitted','cleaned','blocked','retryable_failed','abandoned')),
    lease_owner TEXT,
    lease_expires_at TIMESTAMPTZ,
    worker_run_id UUID REFERENCES stage_worker_runs(id),
    result JSONB,
    cleanup_evidence_refs BIGINT[] NOT NULL DEFAULT '{}',
    terminal_note TEXT,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    row_version BIGINT NOT NULL DEFAULT 0,
    UNIQUE(obligation_id, ordinal)
);

CREATE UNIQUE INDEX cleanup_attempts_one_live
ON cleanup_attempts(obligation_id)
WHERE status IN ('queued','leased','running','submitted','cleaned');

CREATE TABLE cleanup_absence_checks (
    absence_check_id UUID PRIMARY KEY,
    obligation_id UUID NOT NULL REFERENCES cleanup_obligations(obligation_id) ON DELETE RESTRICT,
    cleanup_attempt_id UUID NOT NULL REFERENCES cleanup_attempts(cleanup_attempt_id) ON DELETE RESTRICT,
    operation_id UUID NOT NULL,
    organization_id_at_time UUID NOT NULL,
    scope_snapshot_hash TEXT NOT NULL,
    verifier_worker_run_id UUID NOT NULL REFERENCES stage_worker_runs(id),
    resource_identity_hash TEXT NOT NULL,
    disposition TEXT NOT NULL CHECK (disposition IN ('absent','still_present','inconclusive')),
    evidence_refs BIGINT[] NOT NULL,
    checked_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (cardinality(evidence_refs) > 0),
    UNIQUE(obligation_id, cleanup_attempt_id, verifier_worker_run_id),
    FOREIGN KEY(obligation_id, operation_id, organization_id_at_time, scope_snapshot_hash)
        REFERENCES cleanup_obligations(obligation_id, operation_id, organization_id_at_time, scope_snapshot_hash)
);

CREATE TABLE cleanup_waiver_decisions (
    waiver_id UUID PRIMARY KEY,
    obligation_id UUID NOT NULL REFERENCES cleanup_obligations(obligation_id) ON DELETE RESTRICT,
    operation_id UUID NOT NULL,
    organization_id_at_time UUID NOT NULL,
    scope_snapshot_hash TEXT NOT NULL,
    decision_version BIGINT NOT NULL,
    trusted_user_id TEXT NOT NULL,
    trusted_session_id TEXT NOT NULL,
    reason TEXT NOT NULL,
    evidence_refs BIGINT[] NOT NULL,
    decided_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (length(trim(reason)) > 0),
    CHECK (cardinality(evidence_refs) > 0),
    UNIQUE(obligation_id, decision_version),
    FOREIGN KEY(obligation_id, operation_id, organization_id_at_time, scope_snapshot_hash)
        REFERENCES cleanup_obligations(obligation_id, operation_id, organization_id_at_time, scope_snapshot_hash)
);

ALTER TABLE cleanup_obligations
    ADD COLUMN terminal_waiver_id UUID REFERENCES cleanup_waiver_decisions(waiver_id) ON DELETE RESTRICT;

ALTER TABLE post_exploit_actions
    ADD CONSTRAINT post_exploit_actions_cleanup_obligation_fk
    FOREIGN KEY (cleanup_obligation_id) REFERENCES cleanup_obligations(obligation_id)
    DEFERRABLE INITIALLY DEFERRED;
```

absence evidence 必须由独立 `cleanup_absence_checks` 行持有，不再塞进 cleanup attempt。repo validator 要求 verifier worker 与执行 cleanup 的 worker 不同、absence evidence ids 与 cleanup evidence ids 不重叠、resource identity hash 精确相同、disposition=`absent` 后才能 terminalize `verified_absent`。

### 步骤 3：实现 transaction API

```rust
pub async fn prepare_action_with_obligation(pool: &PgPool, command: PrepareSideEffect) -> Result<PreparedActionWithObligation>;
pub async fn claim_next_obligation(pool: &PgPool, query: CleanupClaimQuery) -> Result<Option<CleanupAttemptRow>>;
pub async fn mark_cleaned(pool: &PgPool, command: MarkCleaned) -> Result<CleanupAttemptRow>;
pub async fn verify_absent_and_terminalize(tx: &mut Transaction<'_, Postgres>, command: VerifyAbsent) -> Result<CleanupObligationRow>;
pub async fn waive_with_residual(tx: &mut Transaction<'_, Postgres>, command: WaiveCleanup, actor: &TrustedDecisionContext) -> Result<CleanupObligationRow>;
pub async fn list_side_effects_missing_obligations(pool: &PgPool, operation_id: Uuid, organization_id_at_time: Uuid) -> Result<Vec<MissingObligationFact>>;
pub async fn list_recovery_work(pool: &PgPool) -> Result<Vec<CleanupRecoveryRow>>;
```

`prepare_action_with_obligation` 内部使用一笔短事务完成 action prepared + obligation + action back-reference，并调用 Memory Core transaction-aware enqueue API；enqueue 追加 `knowledge_outbox_events` 并建立 required `knowledge_projection_deliveries`，本域不直接写 event/delivery/status 表。外部副作用和 cleanup 都不在 transaction 内。idempotency key 重放返回原 action/obligation。若执行结果未知，action/attempt 进入 reconcile 状态，reconciler 先读取 external ref/resource truth，不允许盲目重做。

`list_side_effects_missing_obligations` 使用 `post_exploit_actions LEFT JOIN cleanup_obligations`，覆盖 `prepared|executing|succeeded|uncertain|reconcile_required` 且 `side_effect_class <> 'none'` 的 action；不能只从 obligation 表开始查，否则“根本没创建 obligation”的最危险状态会被漏掉。

### 步骤 4：GREEN 与提交

```bash
cd backend && cargo nextest run -p golish-db cleanup_ --status-level fail --no-tests=fail
just precommit
```

```bash
git add backend/crates/golish-db/migrations/20260712000006_cleanup_obligation_ledger.sql backend/crates/golish-db/src/repo/mod.rs backend/crates/golish-db/src/repo/cleanup_obligations.rs backend/crates/golish-db/src/repo/cleanup_attempts.rs backend/crates/golish-db/src/repo/cleanup_absence_checks.rs backend/crates/golish-db/src/repo/cleanup_waivers.rs backend/crates/golish-post-exploit-app/src/ports.rs backend/crates/golish-post-exploit-app/src/objective_simulation.rs backend/crates/golish-post-exploit-app/src/reconcile.rs
git commit -m "feat(db): create cleanup obligation for every side effect"
```

---

## Task 3：实现 cleanup service、capability 与 absence verifier

**文件：** cleanup-app。

### 步骤 1：写 RED

```rust
#[tokio::test]
async fn cleanup_executes_after_claim_transaction_is_committed() {
    let tracker = TransactionTracker::default();
    let executor = AssertNoOpenTransactionExecutor::new(tracker.clone());
    let app = cleanup_app_with(executor, IndependentAbsenceVerifier::passing()).await;
    let obligation = app.seed_pending_obligation(resource("remote-user:alice")).await;
    let outcome = app.run_one_obligation(app.claim(obligation.id).await.unwrap()).await.unwrap();
    assert_eq!(outcome.status, CleanupObligationStatus::VerifiedAbsent);
    assert_eq!(tracker.open_transactions_during_external_calls(), 0);
}

#[tokio::test]
async fn cleaned_without_independent_absence_check_remains_nonterminal() {
    let app = cleanup_app_with(CleanupExecutorStub::cleaned(vec![91]), AbsenceVerifierStub::inconclusive()).await;
    let obligation = app.seed_pending_obligation(resource("remote-user:alice")).await;
    let outcome = app.run_one_obligation(app.claim(obligation.id).await.unwrap()).await.unwrap();
    assert_eq!(outcome.status, CleanupObligationStatus::Cleaned);
    assert_eq!(app.open_obligation_count().await, 1);
    assert_eq!(app.absence_checks(obligation.id).await[0].disposition, AbsenceDisposition::Inconclusive);
}

#[tokio::test]
async fn retry_and_reconcile_keep_exact_obligation_and_idempotency_key() {
    let executor = UnknownThenObservableAbsentExecutor::default();
    let app = cleanup_app_with(executor.clone(), IndependentAbsenceVerifier::passing()).await;
    let obligation = app.seed_pending_obligation(resource("remote-user:alice")).await;
    let first = app.run_one_obligation(app.claim(obligation.id).await.unwrap()).await.unwrap();
    assert_eq!(first.attempt_status, CleanupAttemptStatus::RetryableFailed);

    let resumed = app.recover_open_obligations().await.unwrap();
    assert_eq!(resumed[0].obligation_id, obligation.id);
    assert_eq!(resumed[0].resource_identity_hash, obligation.resource_identity_hash);
    assert_eq!(executor.mutating_calls(), 1, "reconcile 不得盲目重复 cleanup action");
}

#[tokio::test]
async fn absence_verifier_rejects_wrong_resource_or_reused_evidence() {
    let app = cleanup_test_app().await;
    let attempt = app.seed_cleaned_attempt(resource("remote-user:alice"), vec![92]).await;
    let wrong = absence_result(resource("remote-user:bob"), worker_run_id(9), vec![93]);
    assert_app_code(app.submit_absence(attempt.id, wrong).await, "CLEANUP_ABSENCE_RESOURCE_MISMATCH");

    let reused = absence_result(resource("remote-user:alice"), worker_run_id(9), vec![92]);
    assert_app_code(app.submit_absence(attempt.id, reused).await, "CLEANUP_ABSENCE_EVIDENCE_NOT_INDEPENDENT");
}

#[tokio::test]
async fn recovery_runs_with_post_exploit_and_cleanup_features_disabled() {
    let app = cleanup_app_with_flags(RuntimeFlags { post_exploit_domain: false, cleanup_ui: false }).await;
    let obligation = app.seed_pending_obligation(resource("remote-user:alice")).await;
    app.start_recovery_supervisor().await.unwrap();
    assert_eq!(app.recovery_claimed_ids().await, [obligation.id]);
    assert!(app.new_side_effects_are_disabled());
}
```

### 步骤 2：定义 ports/services

```rust
#[async_trait]
pub trait CleanupExecutor { async fn execute(&self, command: CleanupExecutionCommand) -> Result<CleanupExecutionResult>; }
#[async_trait]
pub trait AbsenceVerifier { async fn verify(&self, command: AbsenceVerificationCommand) -> Result<AbsenceVerificationResult>; }

pub async fn run_one_obligation(&self, claim: CleanupClaim) -> Result<CleanupRunOutcome>;
pub async fn reconcile_uncertain_attempt(&self, attempt_id: Uuid) -> Result<CleanupRunOutcome>;
pub async fn recover_open_obligations(&self) -> Result<Vec<CleanupRunOutcome>>;
```

执行：claim 短事务结束 → external cleanup（传 durable idempotency key）→ cleanup evidence booking → mark cleaned → **不同 worker/run** 的 independent absence check → absence evidence booking → terminal transaction。cleanup executor 和 absence verifier 不得共用 evidence ids；absence verifier 只能验证 exact resource identity/proof requirements，不能执行 cleanup。

恢复 supervisor 不受 `post_exploit_domain`、cleanup UI 或“允许新增动作”开关控制。`app/bootstrap.rs` 在数据库可用后始终查询 `list_recovery_work`；只要存在 pending/running/cleaned/retryable/uncertain work，就拉起 supervisor。开关关闭时仅禁止创建新副作用 action。recovery 先处理 missing-obligation invariant 和 uncertain reconciliation，再 claim cleanup work。

### 步骤 3：注册 capability

```text
cleanup_execute_obligation
cleanup_verify_resource_absent
```

worker objective 只包含 exact obligation/resource/strategy/proof requirements；不得自由扩大资源集合。

### 步骤 4：GREEN 与提交

```bash
cd backend && cargo nextest run -p golish-cleanup-app --status-level fail --no-tests=fail
just precommit
```

```bash
git add backend/Cargo.toml backend/crates/golish-cleanup-app/Cargo.toml backend/crates/golish-cleanup-app/src/lib.rs backend/crates/golish-cleanup-app/src/ports.rs backend/crates/golish-cleanup-app/src/service.rs backend/crates/golish-cleanup-app/src/capabilities.rs backend/crates/golish-cleanup-app/src/absence_verifier.rs backend/crates/golish-cleanup-app/src/reconcile.rs backend/crates/golish-cleanup-app/src/recovery.rs backend/crates/golish-agent-kit/src/harness/stage_capability.rs backend/crates/golish-agent-bridge/src/agent_bridge/config.rs backend/crates/golish/src/app/bootstrap.rs
git commit -m "feat(cleanup): execute and independently verify cleanup obligations"
```

---

## Task 4：建立 Cleanup stage Gate、handoff 与 organization cleanup integration

**文件：** cleanup spec/methodology、gate、artifact_cleanup。

### 步骤 1：写 RED

```rust
#[tokio::test]
async fn cleanup_gate_blocks_open_and_missing_obligation_facts() {
    let base = cleanup_gate_snapshot(operation_id(1), org_id(1)).with_db_loaded(true);
    for status in [CleanupObligationStatus::Pending, CleanupObligationStatus::Running, CleanupObligationStatus::Cleaned] {
        let verdict = cleanup_obligations_terminal(&base.clone().with_obligation(obligation_in(status)));
        assert_block_code(verdict, "CLEANUP_OBLIGATION_NONTERMINAL");
    }

    let missing = base.with_missing_obligation_action(side_effect_action(action_id(9), resource("remote-user:alice")));
    assert_block_code(cleanup_obligations_terminal(&missing), "CLEANUP_SIDE_EFFECT_MISSING_OBLIGATION");
    let read_error = CleanupGateSnapshot::db_error(operation_id(1), org_id(1), "timeout");
    assert_block_code(cleanup_obligations_terminal(&read_error), "CLEANUP_DB_TRUTH_UNAVAILABLE");
}

#[tokio::test]
async fn cleanup_gate_accepts_verified_absent_and_discloses_every_residual() {
    let snapshot = cleanup_gate_snapshot(operation_id(1), org_id(1))
        .with_verified_absent(obligation_id(1), independent_absence_check(vec![101]))
        .with_blocked(obligation_id(2), residual("WAF prevented deletion", vec![102]))
        .with_waived(obligation_id(3), trusted_waiver("user-7", "session-3", vec![103]), residual("owner accepted account", vec![103]));
    let verdict = cleanup_obligations_terminal(&snapshot);
    assert!(verdict.allowed());
    assert_eq!(verdict.residual_obligation_ids(), [obligation_id(2), obligation_id(3)]);
    assert_eq!(verdict.absence_evidence_ids(), [101]);
}

#[tokio::test]
async fn organization_delete_is_blocked_until_every_obligation_is_terminal() {
    let app = organization_cleanup_test_app().await;
    let org = app.seed_org_with_obligation(CleanupObligationStatus::Cleaned).await;
    let result = app.delete_organization(org.id).await;
    assert_app_code(result, "ORG_DELETE_OPEN_CLEANUP_OBLIGATIONS");
    assert!(app.organization_exists(org.id).await);
    assert_eq!(app.audit_evidence_count(org.id).await, 1);

    app.attach_independent_absence_and_terminalize(org.obligation_id).await;
    let deleted = app.delete_organization(org.id).await.unwrap();
    assert_eq!(deleted.invalidated_projection_count, 1);
    assert!(!app.organization_exists(org.id).await);
    assert_eq!(app.cleanup_history_count_for_organization_at_time(org.id).await, 1);
    assert_eq!(app.audit_evidence_count(org.id).await, 1, "org delete 不删除历史 evidence");
}
```

### 步骤 2：Gate op

新增 `cleanup_obligations_terminal`：

- pending/running/cleaned → BLOCK。
- verified_absent → clean terminal。
- blocked/waived → terminal only if residual object + evidence/decision 完整；handoff 必须带 residual refs。
- DB read error → BLOCK。
- `post_exploit_actions LEFT JOIN cleanup_obligations` 查到任何 side-effect action 缺 obligation → BLOCK，即使 obligation 查询本身为空。
- verified_absent 必须关联独立 passed absence check；仅 attempt 上的 cleanup evidence 不足以 PASS。

### 步骤 3：methodology

写明先 clean 后独立 absence verify、重试和 deadline、禁止删除 evidence、waiver 只能由用户命令写入。

### 步骤 4：组织删除

`artifact_cleanup.rs` 在任何删除/解绑前查询 org subtree 的 obligation 和 missing-obligation facts。存在 pending/running/cleaned 或 live attempt 时返回带稳定 `code=ORG_DELETE_OPEN_CLEANUP_OBLIGATIONS` 的 conflict，并列出 obligation ids/count；不得通过终止 lease、自动 waiver 或删除 runtime row 绕过。只有所有 obligation 为 verified_absent，或具备 trusted waiver/blocked residual 的终态后，才可 invalidate knowledge projections 并删除 live organization/target 数据。cleanup 历史以 `organization_id_at_time`、canonical resource snapshot 和 scope hash 保留，不 FK live organization/target，也不靠隐藏 audit anchor 阻止删除；audit evidence、obligation、attempt、absence check、waiver 和 residual 继续保留。

### 步骤 5：GREEN 与提交

```bash
cd backend && cargo nextest run -p golish-agent-kit cleanup_obligations --status-level fail --no-tests=fail
cd backend && cargo nextest run -p golish-recon-app artifact_cleanup --status-level fail --no-tests=fail
just precommit
```

```bash
git add resources/harness/stages/cleanup/spec.json resources/harness/stages/cleanup/methodology.md backend/crates/golish-agent-kit/src/harness/stage_capability.rs backend/crates/golish-agent-kit/src/harness/resources.rs backend/crates/golish-agent-kit/src/harness/stage_spec.rs backend/crates/golish-agent-kit/src/harness/gate/mod.rs backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs backend/crates/golish-agent-kit/src/harness/gate/context_builder.rs backend/crates/golish-recon-app/src/organizations/artifact_cleanup.rs
git commit -m "feat(harness): gate cleanup on verified absence or disclosed residual"
```

---

## Task 5：治理 IPC/UI

**文件：** commands/facade/API/component。

### 步骤 1：写 RED

Backend RED：

```rust
#[tokio::test]
async fn waiver_command_uses_server_trusted_actor_and_rejects_idor_or_stale_version() {
    let app = command_test_app().await;
    let obligation = app.seed_blocked_obligation(org_id(1), row_version(3)).await;
    let actor = TrustedDecisionContext::authenticated_user("user-7", "session-3");

    let sibling = waive_input(obligation.id, org_id(2), 3, "accepted residual", vec![111]);
    assert_command_code(app.waive(actor.clone(), sibling).await, "CLEANUP_OBLIGATION_NOT_FOUND");

    let stale = waive_input(obligation.id, org_id(1), 2, "accepted residual", vec![111]);
    assert_command_code(app.waive(actor.clone(), stale).await, "CLEANUP_STALE_VERSION");

    let accepted = app.waive(actor, waive_input(obligation.id, org_id(1), 3, "accepted residual", vec![111])).await.unwrap();
    assert_eq!(accepted.waiver.trusted_user_id, "user-7");
    assert_eq!(accepted.waiver.trusted_session_id, "session-3");
    assert_eq!(accepted.status, CleanupObligationStatus::WaivedByUser);
}

#[tokio::test]
async fn retry_command_remains_available_when_new_cleanup_actions_are_disabled() {
    let app = command_test_app_with_flags(RuntimeFlags { post_exploit_domain: false, cleanup_ui: true }).await;
    let obligation = app.seed_retryable_obligation(org_id(1)).await;
    let row = app.retry(authenticated_user("user-7"), retry_input(obligation.id, obligation.row_version)).await.unwrap();
    assert_eq!(row.status, CleanupAttemptStatus::Queued);
}
```

Frontend RED：

```tsx
it("renders loading, error, empty, open, residual, and verified-absent states", async () => {
  const api = mockCleanupApi();
  const view = render(<CleanupObligationList operationId="op-1" api={api} />);
  expect(view.getByRole("status")).toHaveTextContent("加载");

  api.rejectList({ code: "CLEANUP_DB_TRUTH_UNAVAILABLE" });
  expect(await view.findByRole("alert")).toHaveTextContent("无法读取清理账本");

  api.resolveList([]);
  expect(await view.findByText("暂无清理义务")).toBeInTheDocument();

  api.resolveList([pendingObligation(), waivedResidual(), verifiedAbsent()]);
  expect(await view.findByText("待清理")).toBeInTheDocument();
  expect(view.getByText("残余风险")).toBeInTheDocument();
  expect(view.getByText("已验证无残留")).toBeInTheDocument();
});

it("requires explicit waiver confirmation and never sends a decider field", async () => {
  const api = mockCleanupApi([blockedObligation({ rowVersion: 4 })]);
  const view = render(<CleanupObligationList operationId="op-1" api={api} />);
  await user.click(await view.findByRole("button", { name: "申请豁免" }));
  await user.type(view.getByLabelText("原因"), "owner accepted residual");
  await user.click(view.getByRole("button", { name: "确认提交" }));
  expect(api.waive).toHaveBeenCalledWith(expect.objectContaining({ expectedRowVersion: 4, reason: "owner accepted residual" }));
  expect(api.waive.mock.calls[0][0]).not.toHaveProperty("decidedBy");
});
```

### 步骤 2：commands

```text
cleanup_list_obligations
cleanup_retry_obligation
cleanup_waive_obligation
```

`cleanup_waive_obligation` 是高风险决策，不能自动倒计时确认；input 只带 obligation id、expected row version、reason、evidence refs，不接受 `decided_by`。command 从登录会话/可信 app context 注入 `TrustedDecisionContext`，模型、tool args、前端字段都不能伪造 waiver actor。retry/list/recovery 不受“禁止新增 post-exploit action”开关影响。

### 步骤 3：GREEN 与提交

```bash
cd backend && cargo nextest run -p golish-agent-app cleanup --status-level fail --no-tests=fail
pnpm exec vitest run frontend/components/Engagement/CleanupObligationList.test.tsx
pnpm typecheck
just precommit
```

```bash
git add backend/crates/golish-agent-app/src/ai/commands/cleanup.rs backend/crates/golish-agent-app/src/ai/commands/mod.rs backend/crates/golish/src/commands_facade/cleanup.rs backend/crates/golish/src/commands_facade/mod.rs backend/crates/golish/src/commands_registry.rs frontend/lib/api/cleanup.ts frontend/components/Engagement/CleanupObligationList.tsx frontend/components/Engagement/CleanupObligationList.test.tsx
git commit -m "feat(ui): expose cleanup obligations and residual decisions"
```

---

## Task 6：包级门禁与文档

```bash
cd backend && cargo nextest run -p golish-cleanup-domain --status-level fail --no-tests=fail
cd backend && cargo nextest run -p golish-cleanup-app --status-level fail --no-tests=fail
cd backend && cargo nextest run -p golish-db cleanup_ --status-level fail --no-tests=fail
cd backend && cargo nextest run -p golish-agent-kit cleanup_ --status-level fail --no-tests=fail
cd backend && cargo nextest run -p golish-agent-app cleanup_ --status-level fail --no-tests=fail
pnpm exec vitest run frontend/components/Engagement/CleanupObligationList.test.tsx
pnpm typecheck
cd backend && cargo clippy -p golish-cleanup-domain -p golish-cleanup-app -p golish-db -p golish-agent-kit -p golish-agent-app -p golish-recon-app --all-targets -- -D warnings
cd backend && cargo fmt --all -- --check
just precommit
```

同步 Task 0 已创建的 cleanup domain/app 卡及 post-exploit、DB、harness、frontend 卡和 INDEX；不要到最后才补新卡。提交：

```bash
git add docs/modules/backend/golish-cleanup-domain.md docs/modules/backend/golish-cleanup-app.md docs/modules/backend/golish-post-exploit-app.md docs/modules/backend/golish-db.md docs/modules/backend/golish-agent-kit.md docs/modules/backend/golish-agent-bridge/agent_bridge.md docs/modules/backend/golish/app.md docs/modules/backend/golish-recon-app/organizations.md docs/modules/frontend/components.md docs/modules/INDEX.md agent-progress.md feature_list.json
git commit -m "docs(cleanup): document obligation and residual safety contract"
```

---

## 不可回滚的不变量

- 已创建 obligation 不能因关闭 feature flag 消失。
- `cleaned` 不能显示为完成。
- blocked/waived 不能伪装成无残留。
- 历史 evidence 不因 cleanup 或 organization deletion 被删除。
- 任何 side-effect action 缺 cleanup obligation 都是 Gate/recovery invariant violation，不能因“obligation 查询为空”而漏过。
- nonterminal obligation 存在时 organization delete 必须 fail closed。
- 关闭 post-exploit、cleanup UI 或新增动作开关，不得关闭既有 obligation 的 recovery、reconcile 和 absence verification。
