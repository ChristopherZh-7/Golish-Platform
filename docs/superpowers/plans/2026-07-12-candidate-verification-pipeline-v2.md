> Superseded by [2026-07-12-candidate-verification-pipeline-v2-corrected.md](./2026-07-12-candidate-verification-pipeline-v2-corrected.md).

# Candidate 审批、逐条验证与 FactDelta 波次 V2 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 把 `attack_candidates` 升级为 DB 权威队列，在确定性扫描后逐 Candidate 审批、逐 CandidateAttempt 验证，并且只由 evidence-backed FactDelta 打开下一攻击波。

**架构：** Candidate 保存 hypothesis/identity；后端 capability classifier 生成 immutable CandidateExecutionPlan；Approval 绑定 exact plan hash/target/capabilities/actions/budget/expiry；Attempt 保存执行与恢复；AttemptEvidence 保存证明归属；CandidateReviewBarrier 持久化 interrupt/resume；Verification Gate 只读 exact wave/unit DB snapshot；Finding 只由 terminalizer 创建；operation-level wave run 汇聚 per-org units 后才移动全局 cursor。外层仍调用 `stage_run`，Verification 内部调度器一次领取一个 Candidate，并先占用 DB execution lane。

**技术栈：** Rust 2021、sqlx/PostgreSQL、Tauri/ts-rs、React/TypeScript、cargo-nextest、Vitest。

**依赖：** P1 Runtime Foundation 已通过 V2Only tests；实施 schema/IPC/真实 exploit 前分别取得用户确认。

---

## 1. 与 2026-07-02 计划的替代关系

保留：StageKind、三阶段 DAG、基础 `AttackCandidate` DTO、formulaic sweep、初版 gate op 名称。

替代：

- deliverable candidate 数组 → DB 权威 Candidate queue。
- gate 前非事务 upsert → final org Gate PASS 后接受 batch。
- generic phase approval → Candidate 生成后逐条 approval。
- deliverable-based Verification Gate → DB-approved Candidate + terminal Attempt Gate。
- in-memory wave cursor → `attack_waves + attack_fact_deltas`。
- verification worker 共用 stage 记忆 → per-CandidateAttempt worker/checkpoint。
- upsert 覆盖 disposition → proposed-only create + CAS transition。

---

## 2. 文件结构

### 新建

- `backend/crates/golish-db/migrations/20260712000002_attack_execution_v2.sql`
- `backend/crates/golish-db/tests/attack_execution_v2_migrations.rs`
- `backend/crates/golish-db/src/repo/attack_candidate_approvals.rs`
- `backend/crates/golish-db/src/repo/candidate_attempts.rs`
- `backend/crates/golish-db/src/repo/attack_fact_deltas.rs`
- `backend/crates/golish-db/src/repo/attack_waves.rs`
- `backend/crates/golish-db/src/repo/attack_execution_lanes.rs`
- `backend/crates/golish-db/src/repo/finding_lineage.rs`
- `backend/crates/golish-agent-kit/src/harness/{attack_execution.rs,verification_truth.rs}`
- `backend/crates/golish-agent-kit/src/harness/candidate_review_barrier.rs`
- `backend/crates/golish-agent-app/src/ai/db_bridge/attack_execution.rs`
- `backend/crates/golish-agent-app/src/ai/commands/attack.rs`
- `backend/crates/golish-agent-app/src/ai/candidate_attempt_submit_tool.rs`
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/candidate_run_call.rs`
- `backend/crates/golish/src/commands_facade/attack.rs`
- `frontend/lib/api/attack.ts`
- `frontend/components/Engagement/{AttackCandidateReview.tsx,AttackCandidateReview.test.tsx,CandidateAttemptRows.tsx,CandidateAttemptRows.test.tsx}`

### 修改

- `backend/crates/golish-db/src/repo/{attack_candidates.rs,mod.rs}`
- `backend/crates/golish-agent-kit/src/harness/{types.rs,chain_wave.rs,operation_flow.rs,mod.rs}`
- `backend/crates/golish-agent-kit/src/harness/{phase_flow.rs,pre_action_authorizer.rs}`
- `backend/crates/golish-agent-kit/src/harness/gate/{rule_engine.rs,context_builder.rs}`
- `backend/crates/golish-agent-kit/src/harness/org_gate.rs`
- `backend/crates/golish-agent-kit/src/db_traits/repo.rs`
- `backend/crates/golish-agent-kit/src/task_orchestrator/{orchestrator.rs,prompts/mod.rs}`
- `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`
- `backend/crates/golish-agent-app/src/ai/{harness_submit_tool.rs,db_bridge/mod.rs,commands/mod.rs}`
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/{mod.rs,stage_run_call.rs,sub_agent_call.rs}`
- `backend/crates/golish-sub-agents/src/defaults/builder/{mod.rs,registry.rs}`
- `backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs`
- `backend/crates/golish-sub-agents/src/{executor_types.rs,defaults/tests.rs}`
- `backend/crates/golish-core/src/{agent_session.rs,events/harness_trace.rs}`
- `backend/crates/golish-app-core/src/background_jobs.rs`
- `backend/crates/golish-pentest/src/output_store/findings.rs`
- `backend/crates/golish-pentest-app/src/pentest_bridge/auth_probe.rs`
- `backend/crates/golish-scan-runner/src/nuclei/runner.rs`
- `backend/crates/golish-events/src/{op_trace/mod.rs,transcript/summarizer.rs}`
- `backend/crates/golish/src/commands_facade/mod.rs`
- `backend/crates/golish/src/commands_registry.rs`
- `resources/harness/stages/{vuln_triage,attack_candidate,verification}/{spec.json,methodology.md}`
- `resources/harness/graph/phases.json`
- `frontend/components/Engagement/StageRunOrgRows.tsx`
- `frontend/components/ToolCallDetailView/ToolCallDetailView.tsx`
- `frontend/services/ai-events/{harness-handlers.ts,harness-handlers.test.ts}`

---

## 0. 开工前置

1. 读 `agent-progress.md`、`feature_list.json`、`docs/modules/INDEX.md` 以及上述每个受影响模块卡；缺卡先按模板补卡，再动代码。
2. 若当前唯一 `in_progress` 不是本父 feature，停止并请用户决定，不抢占槽位。
3. 对 migration、`golish-db`、IPC/ts-rs 变更和任何真实 active scan/exploit 分别取得用户明确授权。
4. 运行 `./init.sh`，预期 exit 0；失败先修基础环境并记录，不进入 Task 1。
5. 运行 `git status --short`。本计划触碰的文件已有用户改动时，先在独立干净 worktree 执行或与用户确认 hunk 边界；禁止覆盖、恢复或顺带暂存既有改动。
6. 每个 commit 前必须跑当前 tree 的 `just precommit` 并记录 exit 0；只用本 Task 明列的精确文件 staging，随后核对 `git diff --cached --name-only`。

---

## Task 1：先建立纯 Candidate/Approval/Attempt/Wave 状态机

**文件：** 新建 `harness/attack_execution.rs` 和 `verification_truth.rs`。

### 步骤 1：写 RED 状态转移表

```rust
#[test]
fn candidate_terminal_state_cannot_be_downgraded() {
    assert_eq!(
        transition_candidate(CandidateDisposition::Verified, CandidateEvent::Approve),
        Err(AttackTransitionError::TerminalCandidate)
    );
}

#[test]
fn attempt_requires_current_approval_and_scope_hash() {
    let result = validate_attempt_start(&candidate(), &stale_approval(), &execution_plan(), "scope-v2", server_now());
    assert_eq!(result, Err(AttackTransitionError::StaleApproval));
}

#[test]
fn approval_is_invalid_when_the_execution_plan_changes() {
    let mut changed = execution_plan();
    changed.allowed_capabilities.insert(CapabilityId::new("pentest.exploit"));
    let result = validate_attempt_start(&candidate(), &approval_for(&execution_plan()), &changed, "scope-v1", server_now());
    assert_eq!(result, Err(AttackTransitionError::CandidatePlanHashMismatch));
}

#[test]
fn verified_attempt_requires_proof_evidence() {
    let result = validate_terminal_attempt(&attempt_result("verified", vec![]));
    assert_eq!(result, Err(AttackTransitionError::MissingProofEvidence));
}
```

运行：

```bash
cd backend && cargo nextest run -p golish-agent-kit attack_execution --no-tests=fail --status-level fail
```

预期：编译失败，新模块/类型不存在。

### 步骤 2：定义稳定领域类型

```rust
pub enum CandidateDisposition { Proposed, Approved, Rejected, Verified, Refuted, Blocked }
pub enum ApprovalDecision { Pending, Approved, Rejected, Revoked, Expired }
pub enum CandidateAttemptStatus { Queued, Leased, Running, WaitingBackground, Submitted, Verified, Refuted, Blocked, RetryableFailed, Abandoned }
pub enum AttackWaveStatus { Synthesizing, AwaitingApproval, Verifying, Consolidating, Closed, Exhausted }
pub enum FactDeltaStatus { Recorded, Accepted, Rejected, Consumed }

pub enum VerificationRiskClass { DeterministicSafe, ActiveSafe, Exploit }

pub struct CandidateExecutionPlan {
    pub target_id: Uuid,
    pub risk_class: VerificationRiskClass,
    pub allowed_capabilities: BTreeSet<CapabilityId>,
    pub allowed_actions: Vec<ActionConstraint>,
    pub proof_criteria: Vec<ProofCriterion>,
    pub budget: CandidateBudget,
    pub policy_version: String,
}
```

`candidate_plan_hash` 是上述结构 canonical JSON 的 SHA-256。`VerificationRiskClass` 由后端 capability classifier 根据 capability/action side-effect metadata 得出；模型传入的 risk/allowed tools 只作 proposal，不能降低分类。`Exploit` 必须人工批准；安全策略自动批准也产生完整 approval row。

实现纯函数：

```rust
pub fn transition_candidate(from: CandidateDisposition, event: CandidateEvent) -> Result<CandidateDisposition, AttackTransitionError>;
pub fn transition_attempt(from: CandidateAttemptStatus, event: AttemptEvent) -> Result<CandidateAttemptStatus, AttackTransitionError>;
pub fn validate_attempt_start(candidate: &CandidateFact, approval: &ApprovalFact, plan: &CandidateExecutionPlan, current_scope_hash: &str, now: DateTime<Utc>) -> Result<(), AttackTransitionError>;
pub fn validate_terminal_attempt(result: &CandidateAttemptResult) -> Result<TerminalDisposition, AttackTransitionError>;
pub fn decide_next_wave(input: WaveConsolidationInput) -> WaveDecision;
```

### 步骤 3：补齐边界测试并 GREEN

完整转移规则：Attempt 允许 `queued→leased→running→waiting_background|submitted→terminal`、`running|waiting_background→retryable_failed|abandoned`；running 可直接 submitted。approval 在 attempt 前 expired 时 Candidate CAS 回 proposed 等待重审；live attempt 被 revoked/expired 时先取消未完成 background job，再 Attempt/Candidate terminalize 为 blocked(reason=`approval_revoked|approval_expired`)；重试耗尽为 blocked(reason=`attempts_exhausted`)。测试还覆盖 rejected 不可执行、queued 也计入 one-live、blocked/refuted evidence role、fuel/depth/total cap。

```bash
cd backend && cargo nextest run -p golish-agent-kit attack_execution --no-tests=fail --status-level fail
```

预期：全部 passed。

### 步骤 4：提交

```bash
just precommit
git add -- backend/crates/golish-agent-kit/src/harness/attack_execution.rs backend/crates/golish-agent-kit/src/harness/verification_truth.rs backend/crates/golish-agent-kit/src/harness/mod.rs
git diff --cached --name-only
git commit -m "feat(harness): define candidate execution state machines"
```

---

## Task 2：新增 Attack Execution V2 schema 与 repo

**文件：** migration、`attack_candidates.rs` 和四个新 repo。

### 步骤 1：写 RED SQL contract tests

```rust
#[test]
fn proposed_upsert_never_overwrites_disposition() {
    assert!(!CREATE_PROPOSED_SQL.contains("disposition = EXCLUDED.disposition"));
}

#[test]
fn candidate_identity_is_operation_org_target_scoped() {
    assert!(CREATE_PROPOSED_SQL.contains("operation_uuid"));
    assert!(CREATE_PROPOSED_SQL.contains("organization_id"));
    assert!(CREATE_PROPOSED_SQL.contains("target_id"));
    assert!(CREATE_PROPOSED_SQL.contains("attack_wave_unit_id"));
    assert!(CREATE_PROPOSED_SQL.contains("source_stage_run_unit_id"));
}

#[test]
fn one_candidate_has_at_most_one_live_attempt() {
    assert!(LIVE_ATTEMPT_INDEX_SQL.contains("WHERE status IN"));
    assert!(LIVE_ATTEMPT_INDEX_SQL.contains("'queued'"));
}
```

运行：

```bash
cd backend && cargo nextest run -p golish-db -E 'test(attack_candidate) | test(candidate_attempt) | test(attack_wave) | test(fact_delta) | test(attack_execution_v2_migration)' --no-tests=fail --status-level fail
```

预期：新表/repo 不存在。

`attack_execution_v2_migrations.rs` 复用 P1 mandatory embedded harness，分别运行空 Candidate 表和带 2026-07-02 legacy rows 的升级；真实执行 migration 后用 hostile INSERT 证明 sibling org/wave/target composite mismatch、duplicate live queued Attempt、duplicate approval version 和双 scheduler lane claim 都被数据库拒绝。禁止环境变量缺失时 skip。

### 步骤 2：扩展 attack_candidates

```sql
ALTER TABLE attack_candidates ADD COLUMN operation_uuid UUID REFERENCES operation_state(operation_id);
ALTER TABLE attack_candidates ADD COLUMN target_id UUID REFERENCES targets(id);
ALTER TABLE attack_candidates ADD COLUMN scope_snapshot_hash TEXT;
ALTER TABLE attack_candidates ADD COLUMN attack_wave_unit_id UUID;
ALTER TABLE attack_candidates ADD COLUMN source_stage_run_unit_id UUID REFERENCES stage_run_units(id);
ALTER TABLE attack_candidates ADD COLUMN execution_plan JSONB;
ALTER TABLE attack_candidates ADD COLUMN candidate_plan_hash TEXT;
ALTER TABLE attack_candidates ADD COLUMN risk_class TEXT;
ALTER TABLE attack_candidates ADD COLUMN row_version BIGINT NOT NULL DEFAULT 0;
ALTER TABLE attack_candidates ADD COLUMN terminal_attempt_id UUID;
ALTER TABLE attack_candidates ADD COLUMN terminal_finding_id UUID REFERENCES findings(id);
ALTER TABLE attack_candidates ADD COLUMN disposition_note TEXT;
ALTER TABLE attack_candidates ADD COLUMN evidence_refs BIGINT[] NOT NULL DEFAULT '{}';
ALTER TABLE attack_candidates DROP CONSTRAINT IF EXISTS attack_candidates_organization_id_fkey;
```

V2 把 existing `organization_id` 当 immutable id-at-time，不再 live-FK/CASCADE；组织删除通过 retention/invalidation workflow，不能抹掉 Candidate/Approval/Attempt 历史。用安全 expression 回填可解析 UUID，非法 legacy id 保持 NULL 并标记 legacy；不要直接 cast 整列。建立：

```sql
CREATE UNIQUE INDEX attack_candidates_v2_identity
ON attack_candidates(
    operation_uuid,
    organization_id,
    COALESCE(target_id, '00000000-0000-0000-0000-000000000000'::uuid),
    hypothesis_hash
)
WHERE operation_uuid IS NOT NULL AND organization_id IS NOT NULL;
```

### 步骤 3：新增生命周期表

```sql
CREATE TABLE attack_candidate_approvals (
    approval_id UUID PRIMARY KEY,
    operation_uuid UUID NOT NULL REFERENCES operation_state(operation_id),
    organization_id UUID NOT NULL,
    candidate_id UUID NOT NULL REFERENCES attack_candidates(candidate_id),
    attack_wave_unit_id UUID NOT NULL,
    target_id UUID NOT NULL REFERENCES targets(id),
    decision TEXT NOT NULL CHECK (decision IN ('pending','approved','rejected','revoked','expired')),
    scope_snapshot_hash TEXT NOT NULL,
    candidate_row_version BIGINT NOT NULL,
    candidate_plan_hash TEXT NOT NULL,
    risk_class TEXT NOT NULL CHECK (risk_class IN ('deterministic_safe','active_safe','exploit')),
    allowed_capabilities TEXT[] NOT NULL,
    allowed_actions JSONB NOT NULL,
    budget JSONB NOT NULL,
    policy_version TEXT NOT NULL,
    request_id TEXT,
    decided_by TEXT,
    reason TEXT,
    decision_version BIGINT NOT NULL,
    is_current BOOLEAN NOT NULL DEFAULT TRUE,
    issued_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,
    decided_at TIMESTAMPTZ
);

CREATE UNIQUE INDEX attack_candidate_current_approval
ON attack_candidate_approvals(candidate_id) WHERE is_current;

CREATE UNIQUE INDEX attack_candidate_approval_versions
ON attack_candidate_approvals(candidate_id, decision_version);

CREATE TABLE candidate_attempts (
    attempt_id UUID PRIMARY KEY,
    candidate_id UUID NOT NULL REFERENCES attack_candidates(candidate_id),
    approval_id UUID NOT NULL REFERENCES attack_candidate_approvals(approval_id),
    operation_uuid UUID NOT NULL REFERENCES operation_state(operation_id),
    organization_id UUID NOT NULL,
    attack_wave_unit_id UUID NOT NULL,
    verification_stage_run_unit_id UUID NOT NULL REFERENCES stage_run_units(id),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 1),
    status TEXT NOT NULL CHECK (status IN ('queued','leased','running','waiting_background','submitted','verified','refuted','blocked','retryable_failed','abandoned')),
    lease_owner TEXT,
    lease_token UUID,
    lease_expires_at TIMESTAMPTZ,
    worker_run_id UUID REFERENCES stage_worker_runs(id),
    message_chain_id UUID REFERENCES message_chains(id),
    scope_snapshot_hash TEXT NOT NULL,
    candidate_plan_hash TEXT NOT NULL,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    terminal_disposition TEXT,
    terminal_note TEXT,
    finding_id UUID REFERENCES findings(id),
    checkpoint JSONB NOT NULL DEFAULT '{}',
    budget JSONB NOT NULL DEFAULT '{}',
    row_version BIGINT NOT NULL DEFAULT 0,
    CHECK (
      (status IN ('queued','retryable_failed','abandoned','verified','refuted','blocked') AND lease_token IS NULL)
      OR (status IN ('leased','running','waiting_background','submitted') AND lease_token IS NOT NULL AND lease_owner IS NOT NULL AND lease_expires_at IS NOT NULL)
    ),
    UNIQUE(candidate_id, ordinal)
);

CREATE UNIQUE INDEX candidate_attempts_one_live
ON candidate_attempts(candidate_id)
WHERE status IN ('queued','leased','running','waiting_background','submitted');

CREATE TABLE candidate_attempt_evidence (
    attempt_id UUID NOT NULL REFERENCES candidate_attempts(attempt_id),
    evidence_audit_id BIGINT NOT NULL REFERENCES audit_log(id),
    role TEXT NOT NULL CHECK (role IN ('observation','proof','refutation','blocker','fact_delta')),
    PRIMARY KEY(attempt_id, evidence_audit_id, role)
);

CREATE TABLE finding_lineage (
    finding_id UUID PRIMARY KEY REFERENCES findings(id),
    operation_uuid UUID NOT NULL REFERENCES operation_state(operation_id),
    organization_id UUID NOT NULL,
    target_id UUID REFERENCES targets(id),
    source_candidate_id UUID NOT NULL UNIQUE REFERENCES attack_candidates(candidate_id),
    source_attempt_id UUID NOT NULL UNIQUE REFERENCES candidate_attempts(attempt_id),
    scope_snapshot_hash TEXT NOT NULL,
    evidence_refs BIGINT[] NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

ALTER TABLE attack_candidates
    ADD CONSTRAINT attack_candidates_terminal_attempt_fk
    FOREIGN KEY (terminal_attempt_id) REFERENCES candidate_attempts(attempt_id);

CREATE TABLE attack_wave_runs (
    attack_wave_id UUID PRIMARY KEY,
    operation_uuid UUID NOT NULL REFERENCES operation_state(operation_id),
    scope_snapshot_id UUID NOT NULL REFERENCES operation_org_scope_snapshots(id),
    wave_index INTEGER NOT NULL CHECK (wave_index >= 0),
    status TEXT NOT NULL CHECK (status IN ('synthesizing','awaiting_approval','verifying','consolidating','closed','exhausted')),
    candidate_stage_execution_id UUID REFERENCES stage_runs(id),
    verification_stage_execution_id UUID REFERENCES stage_runs(id),
    opened_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    closed_at TIMESTAMPTZ,
    row_version BIGINT NOT NULL DEFAULT 0,
    UNIQUE(operation_uuid, wave_index)
);

CREATE TABLE attack_wave_units (
    attack_wave_unit_id UUID PRIMARY KEY,
    attack_wave_id UUID NOT NULL REFERENCES attack_wave_runs(attack_wave_id),
    operation_uuid UUID NOT NULL REFERENCES operation_state(operation_id),
    organization_id UUID NOT NULL,
    scope_snapshot_hash TEXT NOT NULL,
    candidate_stage_run_unit_id UUID REFERENCES stage_run_units(id),
    verification_stage_run_unit_id UUID REFERENCES stage_run_units(id),
    status TEXT NOT NULL CHECK (status IN ('synthesizing','awaiting_approval','verifying','consolidating','closed','exhausted')),
    explicit_no_candidate BOOLEAN NOT NULL DEFAULT FALSE,
    review_closed BOOLEAN NOT NULL DEFAULT FALSE,
    proposed_count INTEGER NOT NULL DEFAULT 0,
    pending_count INTEGER NOT NULL DEFAULT 0,
    approved_ever_count INTEGER NOT NULL DEFAULT 0,
    terminal_count INTEGER NOT NULL DEFAULT 0,
    accepted_delta_count INTEGER NOT NULL DEFAULT 0,
    row_version BIGINT NOT NULL DEFAULT 0,
    UNIQUE(attack_wave_id, organization_id),
    UNIQUE(attack_wave_unit_id, operation_uuid, organization_id, scope_snapshot_hash)
);

ALTER TABLE attack_candidates
    ADD CONSTRAINT attack_candidates_wave_owner_fk
    FOREIGN KEY (attack_wave_unit_id, operation_uuid, organization_id, scope_snapshot_hash)
    REFERENCES attack_wave_units(attack_wave_unit_id, operation_uuid, organization_id, scope_snapshot_hash);

ALTER TABLE attack_candidate_approvals
    ADD CONSTRAINT attack_candidate_approvals_wave_owner_fk
    FOREIGN KEY (attack_wave_unit_id, operation_uuid, organization_id, scope_snapshot_hash)
    REFERENCES attack_wave_units(attack_wave_unit_id, operation_uuid, organization_id, scope_snapshot_hash);

ALTER TABLE candidate_attempts
    ADD CONSTRAINT candidate_attempts_wave_owner_fk
    FOREIGN KEY (attack_wave_unit_id, operation_uuid, organization_id, scope_snapshot_hash)
    REFERENCES attack_wave_units(attack_wave_unit_id, operation_uuid, organization_id, scope_snapshot_hash);

CREATE TABLE attack_fact_deltas (
    delta_id UUID PRIMARY KEY,
    operation_uuid UUID NOT NULL REFERENCES operation_state(operation_id),
    organization_id UUID NOT NULL,
    source_wave_unit_id UUID NOT NULL REFERENCES attack_wave_units(attack_wave_unit_id),
    consumer_wave_unit_id UUID REFERENCES attack_wave_units(attack_wave_unit_id),
    source_candidate_id UUID NOT NULL REFERENCES attack_candidates(candidate_id),
    source_attempt_id UUID NOT NULL REFERENCES candidate_attempts(attempt_id),
    source_finding_id UUID REFERENCES findings(id),
    fact_kind TEXT NOT NULL,
    subject_type TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    change_kind TEXT NOT NULL CHECK (change_kind IN ('created','updated','refuted','new_surface')),
    fact_ref JSONB NOT NULL,
    evidence_ids BIGINT[] NOT NULL,
    dedupe_hash TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('recorded','accepted','rejected','consumed')),
    rejection_reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    consumed_at TIMESTAMPTZ,
    UNIQUE(operation_uuid, organization_id, dedupe_hash)
);

CREATE TABLE attack_execution_lanes (
    lane_key TEXT PRIMARY KEY,
    lease_token UUID,
    lease_owner TEXT,
    candidate_attempt_id UUID REFERENCES candidate_attempts(attempt_id),
    lease_expires_at TIMESTAMPTZ,
    heartbeat_at TIMESTAMPTZ,
    row_version BIGINT NOT NULL DEFAULT 0,
    CHECK (
      (lease_token IS NULL AND lease_owner IS NULL AND candidate_attempt_id IS NULL AND lease_expires_at IS NULL)
      OR (lease_token IS NOT NULL AND lease_owner IS NOT NULL AND candidate_attempt_id IS NOT NULL AND lease_expires_at IS NOT NULL)
    )
);

INSERT INTO attack_execution_lanes(lane_key) VALUES ('global:exploit') ON CONFLICT DO NOTHING;
```

每个 operation-level `attack_wave_runs` 在 frozen snapshot 中为每个 org 创建一条 `attack_wave_units`；没有变化的 org 也写 explicit no-work unit，global cursor 只在所有 units review/consolidation terminal 后移动。补 composite FK/constraint trigger，保证 Candidate→Approval→Attempt→Evidence→FindingLineage 的 operation/org/scope/target/wave/unit 一致；hostile sibling fixtures 必须在 DB 层失败。approval/evidence/lineage 使用 RESTRICT/soft invalidation，禁止 cascade 删除审计历史。

### 步骤 4：实现 repo CAS/lease/transaction

必需 API：

```rust
pub async fn create_proposed_batch_after_gate_pass(tx: &mut Transaction<'_, Postgres>, batch: CandidateBatchWrite) -> Result<Vec<AttackCandidateRow>>;
pub async fn transition_disposition_cas(pool: &PgPool, key: CandidateKey, expected: CandidateDisposition, next: CandidateDisposition, expected_version: i64) -> Result<AttackCandidateRow>;
pub async fn review_wave_candidates(tx: &mut Transaction<'_, Postgres>, command: ReviewCandidateBatch) -> Result<WaveReviewResult>;
pub async fn claim_next_approved_with_lane(pool: &PgPool, query: CandidateClaimQuery) -> Result<Option<ClaimedCandidateAttempt>>;
pub async fn terminalize_attempt(tx: &mut Transaction<'_, Postgres>, command: TerminalizeAttempt) -> Result<TerminalizedCandidate>;
pub async fn create_verified_finding_lineage(tx: &mut Transaction<'_, Postgres>, command: CreateVerifiedFinding) -> Result<FindingLineageRow>;
pub async fn consolidate_operation_wave(tx: &mut Transaction<'_, Postgres>, command: ConsolidateOperationWave) -> Result<OperationWaveDecision>;
pub async fn load_verification_truth(pool: &PgPool, query: VerificationTruthQuery) -> Result<VerificationTruthSnapshot>;
```

`review_wave_candidates` append approval version、CAS Candidate disposition、更新 per-org wave counts/review_closed，全部同事务。`claim_next_approved_with_lane` 使用短事务和 `FOR UPDATE SKIP LOCKED`，同时占用 `global:exploit` DB lane、创建/恢复 exact Attempt、写 worker identity；事务结束后才 dispatch。lane heartbeat/release 与 Attempt lease 联动，两个 org/两个进程也只能有一个 exploit-class Attempt。deterministic-safe policy lane 可在后续独立扩展，MVP 不提高并发。

### 步骤 5：GREEN

```bash
cd backend && cargo nextest run -p golish-db -E 'test(attack_candidate) | test(candidate_attempt) | test(attack_wave) | test(fact_delta) | test(attack_execution_lane) | test(finding_lineage)' --no-tests=fail --status-level fail
```

测试必须包括 sibling org IDOR、scope mismatch、CAS stale、并发 claim、terminal replay、delta dedupe/consume。

### 步骤 6：提交

```bash
just precommit
git add -- backend/crates/golish-db/migrations/20260712000002_attack_execution_v2.sql backend/crates/golish-db/src/repo/attack_candidates.rs backend/crates/golish-db/src/repo/attack_candidate_approvals.rs backend/crates/golish-db/src/repo/candidate_attempts.rs backend/crates/golish-db/src/repo/attack_fact_deltas.rs backend/crates/golish-db/src/repo/attack_waves.rs backend/crates/golish-db/src/repo/attack_execution_lanes.rs backend/crates/golish-db/src/repo/finding_lineage.rs backend/crates/golish-db/src/repo/mod.rs backend/crates/golish-db/tests/attack_execution_v2_migrations.rs
git diff --cached --name-only
git commit -m "feat(db): add candidate approval attempt and wave ledger"
```

---

## Task 3：只在 Attack Candidate final Gate PASS 后接受 Candidate

**文件：** `harness_submit_tool.rs`、org Gate、DB bridge。

### 步骤 1：写 RED

```rust
#[tokio::test]
async fn candidate_batch_is_written_once_only_after_final_gate_pass() {
    let fixture = CandidateSubmitFixture::new().await;
    let blocked = fixture.preview_and_submit(GateVerdict::Block).await;
    assert!(blocked.is_err());
    assert_eq!(fixture.candidate_count().await, 0);
    fixture.fail_after_first_candidate_insert();
    assert!(fixture.preview_and_submit(GateVerdict::Pass).await.is_err());
    assert_eq!(fixture.candidate_count().await, 0, "transaction must roll back partial batch");
    fixture.clear_failure();
    let accepted = fixture.preview_and_submit(GateVerdict::Pass).await.expect("accepted batch");
    let replay = fixture.preview_and_submit(GateVerdict::Pass).await.expect("idempotent replay");
    assert_eq!(accepted.candidate_ids, replay.candidate_ids);
    assert_eq!(fixture.candidate_count().await, accepted.candidate_ids.len());
}

#[tokio::test]
async fn candidate_evidence_must_belong_to_same_operation_org_scope_and_stage_unit() {
    let fixture = CandidateSubmitFixture::new().await;
    let err = fixture.submit_with_evidence(fixture.sibling_org_evidence_id()).await.expect_err("foreign evidence");
    assert!(matches!(err, CandidateAcceptanceError::ForeignEvidence { .. }));
    assert_eq!(fixture.candidate_count().await, 0);
}
```

### 步骤 2：删除 gate 前 persist

移除当前 `persist_candidates_if_any` 的 pre-gate 调用。preview 只验证 DTO、evidence refs、scope、dedupe，不写权威 Candidate。

### 步骤 3：Gate PASS 事务接受

在 per-org Gate PASS transaction 内：

1. 锁定 `stage_run_unit`。
2. 重新验证 scope hash 与 evidence ownership。
3. insert proposed batch。
4. 创建/更新 current AttackWave counts。
5. mark unit passed + publish handoff。

任何一步失败整事务回滚。

### 步骤 4：GREEN 与提交

```bash
cd backend && cargo nextest run -p golish-agent-app -E 'test(candidate_batch_is_written) | test(candidate_evidence_must)' --no-tests=fail --status-level fail
```

```bash
just precommit
git add -- backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs backend/crates/golish-agent-app/src/ai/db_bridge/attack_execution.rs backend/crates/golish-agent-app/src/ai/db_bridge/mod.rs backend/crates/golish-agent-kit/src/harness/org_gate.rs backend/crates/golish-agent-kit/src/harness/gate/context_builder.rs
git diff --cached --name-only
git commit -m "fix(harness): accept candidates only after authoritative gate pass"
```

---

## Task 3A：关闭 scanner 直接 Finding 写路径

**文件：** `resources/harness/stages/{vuln_triage,attack_candidate}/{spec.json,methodology.md}`、`task_orchestrator/prompts/mod.rs`、`golish-pentest/src/output_store/findings.rs`、`golish-pentest-app/src/pentest_bridge/auth_probe.rs`、`golish-scan-runner/src/nuclei/runner.rs`、Verification terminalizer。

### 步骤 1：写 RED

```rust
#[tokio::test]
async fn v2_formulaic_hit_cannot_write_finding_outside_terminalizer() {
    let context = FindingWriteContext::HarnessV2Scan { operation_id: op(), organization_id: org() };
    let hit = nuclei_hit("CVE-EXAMPLE", target());
    let err = persist_finding_from_scan(&context, &hit).await.expect_err("scanner finding write must fail");
    assert_eq!(err.code(), "finding_write_requires_verification_terminalizer");
    assert_eq!(count_findings(op()).await, 0);
    assert_eq!(count_candidate_seed_outcomes(op()).await, 1);
}

#[tokio::test]
async fn deterministic_fast_path_still_uses_approval_attempt_and_terminalizer() {
    let seed = deterministic_candidate_seed();
    let approval = policy_approve(seed, "deterministic-safe-v1").await.unwrap();
    let attempt = run_deterministic_attempt(seed, approval).await.unwrap();
    let finding = verification_terminalizer(attempt).await.unwrap();
    assert_eq!(finding.lineage.source_attempt_id, attempt.attempt_id);
    assert!(!finding.evidence_refs.is_empty());
}
```

### 步骤 2：改 stage contract

- `vuln_triage.findings_allowed=false`。
- formulaic hit 写 `technique_outcome/observation + Candidate seed`。
- `attack_candidate` 继承 observation/outcome，不继承 `vuln_finding`。
- methodology/prompt 删除 “found goes straight to finding”。
- deterministic-safe hit 可走 policy approval + exact Attempt 快路，但 Finding 仍由同一 terminalizer 创建。

### 步骤 3：封住 writer

新增 trusted `FindingWriteAuthority::VerificationTerminalizer { attempt_id }`；V2 harness context 下，`output_store/findings.rs`、auth probe、nuclei runner 的直接 writer 必须拒绝其它 authority。非-harness legacy/general UI 路径保持兼容并明确 trace。执行：

```bash
rg -n "INSERT INTO findings|findings::create|persist.*finding" backend/crates -g '*.rs'
```

把每个命中点列进测试矩阵；不得仅修已知三个文件后停止审计。

### 步骤 4：GREEN 与提交

```bash
cd backend && cargo nextest run -p golish-pentest -p golish-pentest-app -p golish-scan-runner -E 'test(v2_formulaic_hit) | test(deterministic_fast_path)' --no-tests=fail --status-level fail
just precommit
git add -- resources/harness/stages/vuln_triage/spec.json resources/harness/stages/vuln_triage/methodology.md resources/harness/stages/attack_candidate/spec.json resources/harness/stages/attack_candidate/methodology.md backend/crates/golish-agent-kit/src/task_orchestrator/prompts/mod.rs backend/crates/golish-pentest/src/output_store/findings.rs backend/crates/golish-pentest-app/src/pentest_bridge/auth_probe.rs backend/crates/golish-scan-runner/src/nuclei/runner.rs
git diff --cached --name-only
git commit -m "fix(attack): route formulaic hits through candidate verification"
```

---

## Task 4：实现 durable Candidate review 与 operation-flow barrier

**文件：** app commands、commands facade/registry、ts-rs DTO、frontend API、`candidate_review_barrier.rs`、`phase_flow.rs`、`operation_flow.rs`、`execute.rs`、`phases.json`、attack/verification specs。

### 步骤 1：写 backend RED

```rust
#[tokio::test]
async fn candidate_review_is_atomic_plan_bound_and_org_scoped() {
    let fixture = CandidateReviewFixture::two_orgs().await;
    let requests = vec![
        fixture.approve_request(fixture.org_a_candidate()),
        fixture.approve_request(fixture.org_b_candidate()),
    ];
    let err = fixture.review_as_org_a(requests).await.expect_err("sibling candidate");
    assert!(matches!(err, CandidateReviewError::OwnershipMismatch { .. }));
    assert_eq!(fixture.current_approval_count().await, 0);
    let stale = fixture.review_as_org_a(vec![fixture.request_with_stale_plan_hash()]).await;
    assert!(matches!(stale, Err(CandidateReviewError::CandidatePlanChanged { .. })));
}

#[tokio::test]
async fn review_barrier_interrupts_resumes_and_branches_from_db_truth() {
    let fixture = CandidateReviewFixture::single_wave().await;
    assert!(matches!(fixture.barrier().await, CandidateReviewBarrierDecision::Interrupt { .. }));
    fixture.approve_all().await;
    assert_eq!(fixture.restart().await.barrier().await, CandidateReviewBarrierDecision::EnterVerification);
    fixture.reset_with_all_rejected().await;
    assert_eq!(fixture.barrier().await, CandidateReviewBarrierDecision::BailToReporting);
    fixture.reset_with_explicit_no_candidate().await;
    assert_eq!(fixture.barrier().await, CandidateReviewBarrierDecision::BailToReporting);
}

#[tokio::test]
async fn policy_auto_approval_still_persists_exact_plan() {
    let fixture = CandidateReviewFixture::deterministic_safe().await;
    let row = fixture.apply_policy("deterministic-safe-v1").await.unwrap();
    assert_eq!(row.decided_by, "policy:deterministic-safe-v1");
    assert_eq!(row.candidate_plan_hash, fixture.plan_hash());
    assert!(!row.allowed_capabilities.is_empty());
}
```

### 步骤 2：定义 DTO/commands

```rust
#[derive(Serialize, TS)]
pub struct AttackCandidateView {
    pub candidate_id: Uuid,
    pub organization_id: Uuid,
    pub target_id: Uuid,
    pub hypothesis: String,
    pub observation_evidence_refs: Vec<i64>,
    pub risk_class: VerificationRiskClass,
    pub execution_plan: CandidateExecutionPlanView,
    pub candidate_plan_hash: String,
    pub row_version: i64,
    pub disposition: CandidateDisposition,
    pub current_approval: Option<CandidateApprovalView>,
}

#[derive(Deserialize, TS)]
pub struct ReviewAttackCandidateInput {
    pub candidate_id: Uuid,
    pub decision: CandidateReviewDecision,
    pub expected_candidate_row_version: i64,
    pub expected_candidate_plan_hash: String,
    pub scope_snapshot_hash: String,
    pub reason: Option<String>,
}
```

命令：

```text
attack_list_candidates
attack_review_candidates
attack_get_candidate_attempts
attack_get_wave_status
```

按 Tauri 五步：command → facade `pub use` → registry → `frontend/lib/api/attack.ts` → ts-rs export。禁止手改 `frontend/lib/generated/`。

### 步骤 3：实现 DB durable review

UI 批量操作时 transaction 锁住全部 Candidate，逐项验证 operation/org/wave/scope/target/row version/plan hash；任一冲突整批失败并返回带 `code` 的 error。后端 capability classifier 决定 risk 和 manual-vs-policy approval，不能信 DTO risk 标签。

### 步骤 4：接 CandidateReviewBarrier

`attack_candidate` final Gate PASS 只完成 synthesis unit；operation flow 随后查询 exact `attack_wave_runs/units`：pending→持久化 Interrupt，重启从 DB 继续；所有 org review_closed 且 approved_ever_count>0→Verification；所有 org explicit no-candidate 或全部 rejected→Reporting。删除 vuln phase generic `exploit_validation` entry approval 和 Verification 静态 approval，避免在 Candidate 生成前授权未知攻击。

`made_progress` 对 Attack Candidate 不再看 findings/evidence/summary：只读 exact wave review result。V2 operation contract 下完全忽略 deliverable `spawned_candidates`/内存 `chain_wave_seen`。

### 步骤 5：GREEN 与提交

```bash
cd backend && cargo nextest run -p golish-agent-app attack_review --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-kit candidate_review_barrier --no-tests=fail --status-level fail
cd backend && cargo check -p golish
```

```bash
just precommit
git add -- backend/crates/golish-agent-app/src/ai/commands/attack.rs backend/crates/golish-agent-app/src/ai/commands/mod.rs backend/crates/golish/src/commands_facade/attack.rs backend/crates/golish/src/commands_facade/mod.rs backend/crates/golish/src/commands_registry.rs frontend/lib/api/attack.ts backend/crates/golish-agent-kit/src/harness/candidate_review_barrier.rs backend/crates/golish-agent-kit/src/harness/phase_flow.rs backend/crates/golish-agent-kit/src/harness/operation_flow.rs backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs resources/harness/graph/phases.json resources/harness/stages/attack_candidate/spec.json resources/harness/stages/verification/spec.json
git diff --cached --name-only
git commit -m "feat(attack): add durable candidate review API"
```

---

## Task 5：传播 trusted CandidateAttempt identity 和 evidence ownership

**文件：** core agent session、background jobs、runtime/sub-agent、evidence bridge。

### 步骤 1：写 RED

```rust
#[tokio::test]
async fn every_tool_and_background_start_revalidates_the_trusted_plan() {
    let fixture = AttemptIdentityFixture::approved().await;
    let context = fixture.trusted_context();
    assert!(pre_action_authorizer(&context, capability("http.probe"), action("GET /proof")).await.is_ok());
    assert!(matches!(
        pre_action_authorizer(&context, capability("shell"), action("model supplied override")).await,
        Err(AuthorizationError::CapabilityOutsideCandidatePlan { .. })
    ));
    fixture.revoke_approval().await;
    assert!(matches!(
        start_background_job(&context, approved_job()).await,
        Err(AuthorizationError::ApprovalRevoked { .. })
    ));
    assert_eq!(fixture.pending_job_count().await, 0);
}

#[tokio::test]
async fn attempt_evidence_is_exact_and_model_identity_arguments_are_ignored() {
    let fixture = AttemptIdentityFixture::two_sibling_attempts().await;
    let context = fixture.attempt_a_context();
    let result = dispatch_with_model_args(&context, json!({"attempt_id": fixture.attempt_b_id()})).await.unwrap();
    assert_eq!(result.trusted_attempt_id, fixture.attempt_a_id());
    fixture.append_attempt_a_proof().await.unwrap();
    let err = fixture.terminalize_attempt_b_with_attempt_a_proof().await.expect_err("foreign proof");
    assert!(matches!(err, TerminalizeError::EvidenceOwnershipMismatch { .. }));
}
```

### 步骤 2：扩展 trusted context

```rust
pub struct CandidateAttemptIdentity {
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub candidate_id: Uuid,
    pub approval_id: Uuid,
    pub attempt_id: Uuid,
    pub attack_wave_unit_id: Uuid,
    pub verification_stage_run_unit_id: Uuid,
    pub target_id: Uuid,
    pub scope_snapshot_hash: String,
    pub candidate_plan_hash: String,
    pub risk_class: VerificationRiskClass,
    pub allowed_capabilities: BTreeSet<CapabilityId>,
    pub allowed_actions: Vec<ActionConstraint>,
    pub remaining_budget: CandidateBudget,
    pub approval_expires_at: DateTime<Utc>,
    pub policy_version: String,
}
```

把 `Option<CandidateAttemptIdentity>` 放入内部 AgentToolContext/BackgroundJob，而不是模型可见 args。`pre_action_authorizer` 在每次 tool dispatch 和 background job start/retry 前从 DB 重新核对 current approval、plan hash、scope、expiry、capability/action/budget；Prompt/allowlist 只是 UX。revoke/expire transaction 标记未完成 jobs cancelled，live Attempt 按状态机 blocked。

### 步骤 3：evidence append 关联

V2 verifier 使用 compound `append_candidate_attempt_evidence`，在同一 DB transaction 写 evidence ledger row 与 `candidate_attempt_evidence` link；role 由 capability/result parser 决定，不接受模型自由字符串。若现有 writer 无法共享 transaction，先改 bridge API，不采用“紧邻 best effort”。

### 步骤 4：GREEN 与提交

```bash
cd backend && cargo nextest run -p golish-agent-runtime candidate_attempt_identity --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-app candidate_attempt_evidence --no-tests=fail --status-level fail
```

```bash
just precommit
git add -- backend/crates/golish-core/src/agent_session.rs backend/crates/golish-app-core/src/background_jobs.rs backend/crates/golish-agent-kit/src/harness/pre_action_authorizer.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/sub_agent_call.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_dispatch.rs backend/crates/golish-agent-app/src/ai/db_bridge/attack_execution.rs backend/crates/golish-agent-app/src/ai/db_bridge/evidence.rs
git diff --cached --name-only
git commit -m "feat(evidence): bind verifier evidence to exact candidate attempts"
```

---

## Task 6：增加 analyst/verifier specialist 与逐条 scheduler

**文件：** sub-agent registry/prompts、`candidate_run_call.rs`、`stage_run_call.rs`。

### 步骤 1：写 RED

```rust
#[test]
fn analyst_and_verifier_registry_enforces_roles_and_hidden_scheduler() {
    let registry = build_default_registry();
    assert!(registry.get("analyst").is_some());
    assert!(registry.get("verifier").is_some());
    assert!(!main_agent_tool_names().contains(&"candidate_run"));
    assert!(!registry.get("analyst").unwrap().allowed_capabilities().iter().any(|c| c.is_exploit()));
}

#[tokio::test]
async fn scheduler_runs_one_exact_candidate_then_the_next_and_recovers_lease() {
    let fixture = SchedulerFixture::two_approved_candidates().await;
    let first = fixture.claim_from_scheduler("scheduler-a").await.unwrap().unwrap();
    assert_eq!(first.objective.candidate_ids(), vec![first.candidate_id]);
    assert!(fixture.claim_from_scheduler("scheduler-b").await.unwrap().is_none(), "global exploit lane is held");
    fixture.expire_leases(first.attempt_id).await;
    let resumed = fixture.claim_from_scheduler("scheduler-b").await.unwrap().unwrap();
    assert_eq!(resumed.attempt_id, first.attempt_id);
    fixture.terminalize_verified(resumed).await.unwrap();
    let second = fixture.claim_from_scheduler("scheduler-b").await.unwrap().unwrap();
    assert_ne!(second.candidate_id, first.candidate_id);
}

#[tokio::test]
async fn retryable_failure_creates_next_ordinal_without_terminalizing_candidate() {
    let fixture = SchedulerFixture::one_candidate().await;
    let first = fixture.claim_from_scheduler("scheduler").await.unwrap().unwrap();
    fixture.record_retryable_failure(first.attempt_id).await.unwrap();
    assert_eq!(fixture.candidate_disposition().await, CandidateDisposition::Approved);
    let retry = fixture.claim_from_scheduler("scheduler").await.unwrap().unwrap();
    assert_eq!(retry.ordinal, first.ordinal + 1);
}
```

### 步骤 2：注册 specialist

- `analyst`：只能生成 Candidate proposal；没有 exploit tool。
- `verifier`：一次只接收 candidate_id/attempt_id/approval_id/wave+unit/exact target/hypothesis/immutable plan hash/proof criterion/allowed capabilities/actions/budget/evidence refs。

verifier 的唯一终止工具：`submit_candidate_attempt`。

### 步骤 3：实现 scheduler

`stage_run` 在 generic `StageSpec.specialist` guard **之前**先判断 active stage + operation contract；Verification V2 进入内部 scheduler，因此当前 spec 即使没有 generic specialist 也不会被提前拒绝：

```rust
while let Some(attempt) = repo.claim_next_approved_with_lane(query).await? {
    let result = run_exact_verifier(attempt.identity()).await;
    submit_or_record_retry(result).await?;
}
```

claim 与占用 `global:exploit` lane 在同一短事务完成，事务在 dispatch 前结束；进程内 semaphore 只减少竞争，不作为并发正确性边界。heartbeat 同时续 Attempt 和 lane；terminal/retry/abandon 原子释放 lane。

### 步骤 4：实现 submit tool validator

`submit_candidate_attempt` 不接受 candidate identity 参数；从 trusted context 读取。验证 terminal disposition、evidence roles、target、scope、approval current、background jobs terminal 后再 terminalize。

### 步骤 5：GREEN 与提交

```bash
cd backend && cargo nextest run -p golish-sub-agents -E 'test(analyst) | test(verifier)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-runtime candidate --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-app candidate_attempt_submit --no-tests=fail --status-level fail
```

```bash
just precommit
git add -- backend/crates/golish-sub-agents/src/defaults/builder/mod.rs backend/crates/golish-sub-agents/src/defaults/builder/registry.rs backend/crates/golish-sub-agents/src/defaults/prompts/execution_planning.rs backend/crates/golish-sub-agents/src/executor_types.rs backend/crates/golish-sub-agents/src/defaults/tests.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/candidate_run_call.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run_call.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs backend/crates/golish-agent-app/src/ai/candidate_attempt_submit_tool.rs
git diff --cached --name-only
git commit -m "feat(attack): verify approved candidates one attempt at a time"
```

---

## Task 7：把 Verification Gate 改成 DB-authoritative

**文件：** gate context/rule engine/org gate/verification truth。

### 步骤 1：写 RED

```rust
#[tokio::test]
async fn gate_uses_exact_wave_snapshot_and_ignores_empty_deliverable_candidates() {
    let fixture = VerificationGateFixture::one_approved_unresolved().await;
    let verdict = fixture.evaluate_with_deliverable_candidates(vec![]).await;
    assert!(matches!(verdict, GateVerdict::Block { code: "approved_candidate_unresolved", .. }));
    fixture.terminalize_without_proof().await;
    let missing_proof = fixture.evaluate().await;
    assert!(matches!(missing_proof, GateVerdict::Block { code: "verified_candidate_missing_proof", .. }));
}

#[tokio::test]
async fn zero_candidate_pass_requires_exact_existing_review_closed_no_candidate_wave() {
    let explicit = VerificationTruthSnapshot::explicit_no_candidate_fixture();
    assert_eq!(candidate_disposition_complete(&explicit), GateVerdict::Pass);
    for invalid in [
        VerificationTruthSnapshot::missing_wave_fixture(),
        VerificationTruthSnapshot::review_open_fixture(),
        VerificationTruthSnapshot::wrong_stage_unit_fixture(),
        VerificationTruthSnapshot::filter_mismatch_empty_fixture(),
    ] {
        assert!(matches!(candidate_disposition_complete(&invalid), GateVerdict::Block { .. }));
    }
}

#[tokio::test]
async fn verification_truth_db_error_is_unavailable_not_empty() {
    let context = build_gate_context(Err(anyhow!("db unavailable"))).await;
    assert!(context.verification_truth.is_none());
    assert!(matches!(evaluate_verification_gate(&context), GateVerdict::Block { code: "verification_truth_unavailable", .. }));
}
```

### 步骤 2：扩展 GateContext

```rust
pub struct VerificationTruthSnapshot {
    pub operation_id: Uuid,
    pub organization_id: Uuid,
    pub scope_snapshot_hash: String,
    pub attack_wave_id: Uuid,
    pub attack_wave_unit_id: Uuid,
    pub verification_stage_run_unit_id: Uuid,
    pub review_closed: bool,
    pub explicit_no_candidate: bool,
    pub proposed_count: u32,
    pub pending_count: u32,
    pub approved_ever_count: u32,
    pub terminal_count: u32,
    pub candidates_approved_in_wave: Vec<VerificationCandidateFact>,
    pub read_watermark: VerificationReadWatermark,
}

pub struct VerificationCandidateFact {
    pub candidate_id: Uuid,
    pub disposition: CandidateDisposition,
    pub terminal_attempt: Option<TerminalAttemptFact>,
}

pub verification_truth: Option<VerificationTruthSnapshot>
```

`None` 表示 DB truth 未成功加载，必须 BLOCK。Snapshot query 以 operation/org/scope/wave/unit/generation 全键读取“本 wave 曾经 approved”的 Candidate，包括当前已变成 verified/refuted/blocked 的行，不能只筛 disposition=`approved`。零集合只有 exact wave 存在、review_closed、pending=0、approved_ever=0 且 explicit_no_candidate=true 时可 PASS；其它空结果均 BLOCK。

### 步骤 3：替换 rule op 实现

`candidate_disposition_complete` 只读上述 DB snapshot，不读取 deliverable candidates。verified/refuted/blocked 调用 evidence role validator；read watermark 在 Gate commit 前复查，变化则 BLOCK/retry。

### 步骤 4：Finding terminalizer

verified Attempt 在一笔 transaction：

1. 验证 proof evidence。
2. 创建/幂等复用 Finding。
3. 写 finding lineage candidate/attempt。
4. terminalize Attempt。
5. CAS Candidate→verified。
6. proposal FactDelta。

### 步骤 5：GREEN 与提交

```bash
cd backend && cargo nextest run -p golish-agent-kit candidate_disposition --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-app verification_truth --no-tests=fail --status-level fail
```

```bash
just precommit
git add -- backend/crates/golish-agent-kit/src/harness/verification_truth.rs backend/crates/golish-agent-kit/src/harness/gate/context_builder.rs backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs backend/crates/golish-agent-kit/src/harness/org_gate.rs backend/crates/golish-agent-app/src/ai/db_bridge/attack_execution.rs
git diff --cached --name-only
git commit -m "fix(gate): verify candidates from database terminal truth"
```

---

## Task 8：实现 FactDelta consolidation 与 durable wave cursor

**文件：** `attack_fact_deltas.rs`、`attack_waves.rs`、chain wave/operation flow/orchestrator。

### 步骤 1：写 RED

```rust
#[tokio::test]
async fn operation_wave_waits_for_every_org_and_replay_opens_exactly_one_next_wave() {
    let fixture = WaveFixture::two_orgs().await;
    fixture.accept_delta_for_org_a().await;
    fixture.inject_deliverable_spawned_candidates_for_org_b();
    assert_eq!(fixture.consolidate_org_a().await.unwrap(), OperationWaveDecision::WaitForOtherUnits);
    fixture.close_org_b_as_no_work().await;
    let first = fixture.consolidate_operation().await.unwrap();
    let replay = fixture.consolidate_operation().await.unwrap();
    assert_eq!(first, replay);
    assert!(matches!(first, OperationWaveDecision::OpenNext { wave_index: 1, .. }));
    assert_eq!(fixture.next_wave_unit_count().await, 2, "every scoped org gets a unit");
    assert_eq!(fixture.consumed_delta_count().await, 1);
    assert_eq!(fixture.deliverable_spawned_candidate_count_used(), 0);
}

#[tokio::test]
async fn foreign_delta_and_fuel_exhaustion_never_reopen_cursor() {
    let fixture = WaveFixture::at_fuel_limit().await;
    assert!(matches!(fixture.accept_sibling_org_delta().await, Err(FactDeltaError::OwnershipMismatch { .. })));
    let decision = fixture.consolidate_operation().await.unwrap();
    assert!(matches!(decision, OperationWaveDecision::Exhausted { .. }));
    assert!(!fixture.residual_risks().await.is_empty());
    assert_eq!(fixture.operation_cursor(), StageKind::Reporting);
}
```

### 步骤 2：验证 FactDelta

validator 必须解析 canonical_fact_ref、source Attempt、source wave unit、evidence ownership、scope hash；无变化/重复/越权写 `rejected` + reason。worker/deliverable 不能直接创建 accepted delta 或移动 cursor。

### 步骤 3：原子 open/consume

所有 per-org wave units review/verification/consolidation terminal 后，operation-level 短事务：

```text
lock current wave
lock every scoped wave unit
require every unit terminal, including explicit no-work units
select accepted unconsumed deltas by exact source wave unit
apply fuel/depth/candidate caps
insert one next attack_wave_run and one unit per frozen org
mark selected deltas consumed with exact consumer_wave_unit_id
close current wave
persist the single operation cursor decision
```

没有 accepted delta 时 close 并走 DAG next，不覆写 DAG edge。

### 步骤 4：GREEN 与提交

```bash
cd backend && cargo nextest run -p golish-db -E 'test(fact_delta) | test(attack_wave)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-kit chain_wave --no-tests=fail --status-level fail
```

```bash
just precommit
git add -- backend/crates/golish-db/src/repo/attack_fact_deltas.rs backend/crates/golish-db/src/repo/attack_waves.rs backend/crates/golish-agent-kit/src/harness/chain_wave.rs backend/crates/golish-agent-kit/src/harness/operation_flow.rs backend/crates/golish-agent-kit/src/task_orchestrator/orchestrator.rs backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs
git diff --cached --name-only
git commit -m "feat(attack): reopen candidate waves from accepted fact deltas"
```

---

## Task 9：实现 Candidate review/Attempt UI 与 trace

**文件：** frontend components/API/store，core/events/transcript。

### 步骤 1：写 frontend RED

```tsx
it("renders loading then the DB-backed empty state", async () => {
  const pending = deferred<AttackCandidateView[]>();
  const api = fakeAttackApi({ listCandidates: vi.fn(() => pending.promise) });
  render(<AttackCandidateReview operationId="op-1" api={api} />);
  expect(screen.getByRole("status")).toHaveTextContent("加载候选");
  pending.resolve([]);
  expect(await screen.findByText("当前波次没有攻击候选")).toBeInTheDocument();
});

it("submits one versioned decision per selected candidate", async () => {
  const candidates = [candidate({ id: "c1", version: 3 }), candidate({ id: "c2", version: 7 })];
  const review = vi.fn().mockResolvedValue({ reviewed: 2 });
  render(<AttackCandidateReview operationId="op-1" api={fakeAttackApi({ candidates, review })} />);
  await userEvent.click(await screen.findByLabelText("选择 c1"));
  await userEvent.click(screen.getByLabelText("选择 c2"));
  await userEvent.click(screen.getByRole("button", { name: "批准所选候选" }));
  expect(review).toHaveBeenCalledWith(expect.arrayContaining([
    expect.objectContaining({ candidateId: "c1", expectedCandidateRowVersion: 3 }),
    expect.objectContaining({ candidateId: "c2", expectedCandidateRowVersion: 7 }),
  ]));
});

it("refetches after a plan-version conflict", async () => {
  const api = fakeAttackApi({ reviewErrorCode: "ATTACK_CANDIDATE_PLAN_CHANGED" });
  render(<AttackCandidateReview operationId="op-1" api={api} />);
  await approveFirstCandidate();
  expect(await screen.findByText("候选计划已变化，请重新审核")).toBeInTheDocument();
  expect(api.listCandidates).toHaveBeenCalledTimes(2);
});

it("renders exact attempt status and evidence links", () => {
  render(<CandidateAttemptRows attempts={[attempt({ ordinal: 2, status: "verified", evidenceIds: [41] })]} />);
  expect(screen.getByText("第 2 次验证")).toBeInTheDocument();
  expect(screen.getByText("已验证")).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "证据 #41" })).toBeInTheDocument();
});
```

```bash
pnpm exec vitest run frontend/components/Engagement/AttackCandidateReview.test.tsx frontend/components/Engagement/CandidateAttemptRows.test.tsx
```

预期：组件不存在/测试失败。

### 步骤 2：实现 UI

- Candidate list 只从 `attack_list_candidates` 读取。
- loading/error/empty 三态齐全。
- 批量 approve/reject 发送逐 candidate version/hash。
- conflict 后重新 fetch，不乐观覆盖 DB。
- Attempt row 显示 exact candidate、ordinal、status、terminal reason、evidence links。

### 步骤 3：新增 trace

```text
candidate_attempt_progress
attack_wave_progress
```

事件只含 ids/status/count/watermark，不含 secret/payload。更新 handler/store 与 transcript summarizer。

### 步骤 4：GREEN 与提交

```bash
pnpm exec vitest run frontend/components/Engagement/AttackCandidateReview.test.tsx frontend/components/Engagement/CandidateAttemptRows.test.tsx frontend/services/ai-events/harness-handlers.test.ts
pnpm typecheck
```

```bash
just precommit
git add -- frontend/lib/api/attack.ts frontend/components/Engagement/AttackCandidateReview.tsx frontend/components/Engagement/AttackCandidateReview.test.tsx frontend/components/Engagement/CandidateAttemptRows.tsx frontend/components/Engagement/CandidateAttemptRows.test.tsx frontend/components/Engagement/StageRunOrgRows.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.tsx frontend/services/ai-events/harness-handlers.ts frontend/services/ai-events/harness-handlers.test.ts backend/crates/golish-core/src/events/harness_trace.rs backend/crates/golish-events/src/op_trace/mod.rs backend/crates/golish-events/src/transcript/summarizer.rs
git diff --cached --name-only
git commit -m "feat(ui): review candidates and track exact verification attempts"
```

---

## Task 10：stage spec、端到端回归与模块卡

**文件：** stage resources、module docs、feature/progress。

### 步骤 1：统一审批时机

资源合同改为：

```text
active_scan approval -> vuln_triage
attack_candidate synthesis -> candidate review
approved candidates -> verification
```

`verification` 不再依赖“进入整个 vuln phase 前批准未知 exploit”。methodology 明确 analyst 不攻击、verifier 一次一个 Candidate。

### 步骤 2：集成测试矩阵

至少实现：

```rust
#[tokio::test]
async fn deterministic_hit_runs_candidate_approval_attempt_and_finding_lineage() {
    let fixture = AttackPipelineFixture::authorized_single_org().await;
    fixture.run_formulaic_hit().await.unwrap();
    assert_eq!(fixture.finding_count().await, 0);
    let candidate = fixture.only_candidate().await;
    let approval = fixture.policy_approve(candidate.candidate_id).await.unwrap();
    let attempt = fixture.run_verification().await.unwrap();
    assert_eq!(attempt.approval_id, approval.approval_id);
    let finding = fixture.only_finding().await;
    assert_eq!(finding.lineage.source_attempt_id, attempt.attempt_id);
}

#[tokio::test]
async fn multi_org_restart_and_two_delta_waves_keep_exact_lineage() {
    let fixture = AttackPipelineFixture::two_orgs().await;
    let running = fixture.start_org_a_attempt().await.unwrap();
    fixture.kill_and_restart().await;
    assert_eq!(fixture.resumed_attempt_id().await, running.attempt_id);
    assert!(fixture.try_attach_org_a_proof_to_org_b().await.is_err());
    fixture.finish_wave_with_delta("a-to-b").await.unwrap();
    fixture.finish_wave_with_delta("b-to-c").await.unwrap();
    assert_eq!(fixture.attack_wave_count().await, 3);
    assert_eq!(fixture.consumed_delta_count().await, 2);
    assert!(fixture.all_findings_have_exact_org_attempt_lineage().await);
}

#[tokio::test]
async fn fuel_limit_terminates_with_residual_instead_of_looping() {
    let fixture = AttackPipelineFixture::at_configured_limits().await;
    fixture.propose_one_more_delta().await.unwrap();
    assert!(matches!(fixture.consolidate().await.unwrap(), OperationWaveDecision::Exhausted { .. }));
    assert_eq!(fixture.operation_stage(), StageKind::Reporting);
    assert!(!fixture.residual_risks().await.is_empty());
}
```

### 步骤 3：包级门禁

```bash
cd backend && cargo nextest run -p golish-db -E 'test(attack_candidate) | test(candidate_attempt) | test(attack_wave) | test(fact_delta) | test(attack_execution_lane) | test(finding_lineage)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-kit -E 'test(attack_execution) | test(candidate_disposition) | test(candidate_review_barrier) | test(chain_wave)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-app attack --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-runtime candidate --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-sub-agents -E 'test(analyst) | test(verifier)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-pentest -p golish-pentest-app -p golish-scan-runner -E 'test(v2_formulaic_hit) | test(deterministic_fast_path)' --no-tests=fail --status-level fail
pnpm exec vitest run frontend/components/Engagement/AttackCandidateReview.test.tsx frontend/components/Engagement/CandidateAttemptRows.test.tsx frontend/services/ai-events/harness-handlers.test.ts
pnpm typecheck
cd backend && cargo clippy -p golish-db -p golish-agent-kit -p golish-agent-app -p golish-agent-runtime -p golish-sub-agents -p golish-core -p golish-events -p golish --all-targets -- -D warnings
cd backend && cargo fmt --all -- --check
just precommit
```

预期：全部 exit 0。

### 步骤 4：更新模块卡和提交

更新受影响 `docs/modules/backend/**` 与 `docs/modules/frontend/**`、索引、feature/progress。

```bash
just precommit
git add -- resources/harness/stages/vuln_triage/spec.json resources/harness/stages/vuln_triage/methodology.md resources/harness/stages/attack_candidate/spec.json resources/harness/stages/attack_candidate/methodology.md resources/harness/stages/verification/spec.json resources/harness/stages/verification/methodology.md resources/harness/graph/phases.json docs/modules/INDEX.md docs/modules/backend/golish-db/repo.md docs/modules/backend/golish-agent-kit/harness.md docs/modules/backend/golish-agent-kit/task_orchestrator.md docs/modules/backend/golish-agent-runtime/agentic_loop.md docs/modules/backend/golish-agent-app/ai.md docs/modules/backend/golish-sub-agents/defaults.md docs/modules/backend/golish-sub-agents/executor.md docs/modules/frontend/components.md docs/modules/frontend/lib.md agent-progress.md feature_list.json
git diff --cached --name-only
git commit -m "docs(attack): complete candidate execution V2 rollout"
```

---

## 本计划完成后仍不做的事

- 不把 Candidate/Attempt prose 自动提升到长期知识；由 P3 完成。
- 不用 KG/RAG 决定 Candidate Gate；P4/P5 只提供 prior。
- 不执行 post-exploit action；由 P6/P7 定义 canonical domain 和 cleanup。
- 不在未获授权的 workspace 上运行真实扫描或 exploit。
