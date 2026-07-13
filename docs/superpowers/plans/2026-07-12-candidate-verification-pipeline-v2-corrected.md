# Candidate 审批、逐条验证与 FactDelta 波次 V2 修正版实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。

**目标：** 在修正版运行期记忆底座上，把扫描 observation 转成可审查的 Candidate 工作队列，逐 CandidateAttempt 做计划绑定、单 lane、可恢复验证，并且只让 evidence-backed terminal result 生成 Finding 或打开下一波。

**架构：** 本计划依赖 `docs/superpowers/plans/2026-07-12-runtime-memory-foundation-corrected.md` 已完成：P2 只引用其 frozen `scope_snapshot_id`、`stage_execution_id`、`stage_run_unit_id`、trusted `deliverable_submission_id` 和 `stage_worker_runs`。Candidate/Approval/Attempt/FactDelta 使用冻结的 operation/org/target identity；live target 只保留 `ON DELETE SET NULL` 的可选引用。`stage_worker_runs` 是 Attempt 唯一 lease/checkpoint owner，DB execution lane 只做全局副作用互斥；MVP verifier 全部前台执行。旧攻击流水线与 V2 的选择由 operation 创建时冻结的 `attack_execution_contract` 决定，切流是本计划最后一个任务。

**技术栈：** Rust 2021、sqlx/PostgreSQL、Tauri 2、ts-rs、React 19、TypeScript 6、Vitest、cargo-nextest。

### 全局 migration 编号 ledger

| 数字前缀 | 唯一归属 |
|---|---|
| `20260712000001` | P1 runtime-memory foundation |
| `20260712000002` | P1 runtime-memory V2 cutover |
| `20260712000003` | Trusted operator principal |
| `20260712000004` | P2 attack-execution V2 foundation |
| `20260712000005` | P2 attack-execution V2 cutover |
| `20260712000006` | Memory Fabric core |
| `20260712000007` | Structured temporal graph |
| `20260712000008` | Post-Exploit core |
| `20260712000009` | Cleanup obligation ledger |
| `20260712000010` | Cleanup closeout |
| `20260712000011` | Reporting read model |

开工及新建任何 migration 前先执行 numeric-prefix 冲突预检：

```bash
duplicate_prefixes="$(find backend/crates/golish-db/migrations -maxdepth 1 -type f -name '*.sql' -exec basename {} \; | cut -d_ -f1 | sort | uniq -d)"
test -z "$duplicate_prefixes" || { printf 'duplicate migration numeric prefix(es):\n%s\n' "$duplicate_prefixes"; exit 1; }
```

---

## 0. 计划地位、前置条件与禁止越线

本文件是 `docs/superpowers/plans/2026-07-12-candidate-verification-pipeline-v2.md` 的修正版执行依据；不修改或删除旧计划。设计依据仍是 `docs/design/2026-07-12-runtime-memory-candidate-pipeline-v2.md`，当旧计划与本文件冲突时，以本文件为准。

开始 P2 代码前必须同时满足：

1. `docs/superpowers/plans/2026-07-12-runtime-memory-foundation-corrected.md` 全部任务完成并有 package gate 证据。
2. P1 已提供 `RuntimeMemoryContract::{Legacy,DualWriteReadLegacy,DualWriteReadV2Fallback,V2Only}`，并把值冻结在 `operation_state.runtime_memory_contract`；运行中不可改；P1 的 `20260712000002_runtime_memory_v2_cutover.sql` 已使之后创建的 operation 具备 P2 的 runtime 前置。
3. P1 已提供 `project_scopes`、`operation_scope_decisions`、`operation_org_scope_snapshots`、`operation_org_scope_units`、`stage_run_units`、`stage_worker_runs`、`stage_deliverable_submissions`、`stage_handoffs`。
4. P2 的 V2 read/execute 只允许在 `runtime_memory_contract = 'v2_only'` 的 operation 上启用；legacy operation 继续走旧路径，不能混读 checkpoint、scope 或 gate truth。
5. `feature_list.json` 只有 `runtime-memory-candidate-pipeline-v2-2026-07-12` 为 `in_progress`，`agent-progress.md` 已记录用户对 additive migration、`golish-db`、IPC 和前端链路的授权。
6. 不运行真实扫描、exploit 或外部 API。计划中的 verifier 集成测试只使用 fake executor、fixture DB 和本地纯函数。
7. P1 未提交或未归属的改动必须先由 P1 收口；P2 不覆盖 `backend/crates/golish-agent-kit/src/harness/runtime_memory_contract.rs`、`backend/crates/golish-db/src/repo/runtime_memory_tx.rs` 等 P1 工作文件中的未知 diff。
8. Task 0-11 的行为变化全部按 persisted `attack_execution_contract` 分支；stage JSON 只做 additive V2 声明，legacy 字段和旧 operation 行为保留。只有 Task 12 推进 deployment singleton 后，新 operation 才走 V2Only。

### 0.1 Operation 级 rollout contract

在 P2 migration 中新增并冻结：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttackExecutionContract {
    Legacy,
    DualWriteReadLegacy,
    DualWriteReadV2Fallback,
    V2Only,
}
```

语义固定为：

| contract | 写路径 | 权威读 | 是否执行 V2 verifier |
|---|---|---|---|
| `legacy` | legacy | legacy | 否 |
| `dual_write_read_legacy` | final Gate PASS 后同事务写 V2 + legacy mirror | legacy；异步比较 V2 | 否 |
| `dual_write_read_v2_fallback` | final Gate PASS 后同事务写 V2 + legacy mirror | 整条 V2；缺失时整条 legacy | 否，仅 shadow scheduler |
| `v2_only` | V2 | V2，缺失即 BLOCK | 是 |

`attack_execution_contract='v2_only'` 还必须满足同一 operation 的 `runtime_memory_contract='v2_only'`。P2 新增 `attack_execution_rollout` singleton 作为 deployment default；operation 创建事务读取并冻结它。P1 的 trigger 保护 `runtime_memory_contract`，P2 另建 `operation_attack_contract_immutable` trigger 保护 `attack_execution_contract`。禁止按进程环境变量在 operation 中途切流。

### 0.2 本计划修正的硬不变量

| 编号 | 修正后的不变量 |
|---|---|
| P2-I1 | 旧 `uq_attack_candidates_op_target_hash` 只约束 `operation_uuid IS NULL` 的 legacy 行；V2 identity 必须包含冻结 org，sibling org 可拥有同 target/hash。 |
| P2-I2 | retained audit rows 不依赖 live organization FK；live target 仅为 nullable `ON DELETE SET NULL`，冻结 target type/value/hash 永远保留。 |
| P2-I3 | Candidate→Approval→Attempt→Evidence→FindingLineage 的 operation/scope/org/wave/unit/target identity 由显式 composite FK 或 constraint trigger 在 DB 层约束。 |
| P2-I4 | evidence 不存 `BIGINT[]`；Candidate、work-item decision、Attempt、FactDelta、residual risk 各用 join table，插入时校验 `audit_log.audit_role='evidence'`、`run_id`、org 和 target ownership。 |
| P2-I5 | 每个 server-seeded reasoning work item 必须终结为 `candidate` 或带 reason/evidence 的 `no_candidate`；空 `candidates[]` 不能 vacuous PASS。 |
| P2-I6 | formulaic scanner 只能写 observation seed、technique outcome 和 evidence；Candidate 只在 `attack_candidate` final Gate PASS transaction 中被接受。 |
| P2-I7 | `stage_worker_runs` 是 CandidateAttempt 唯一 lease/checkpoint owner；Attempt 表没有 lease/checkpoint 字段。lane heartbeat 与 worker heartbeat 同一事务、固定锁序。 |
| P2-I8 | MVP verifier 强制 foreground；candidate context 下 `background=true`、`wait_for_background_jobs` 和 process-only job id 全部拒绝。 |
| P2-I9 | `golish-core::AgentToolContext` 只携带 opaque IDs/hash，不依赖 agent-kit capability/action 类型；runtime 每次 action 前从 DB 重载 immutable plan。 |
| P2-I10 | V2 harness 中只有 Verification terminalizer 能创建 Finding；所有 scanner/bridge/raw writer 都经统一 repo authority，ownership guard 阻止新增旁路 SQL。 |
| P2-I11 | review barrier、resume wakeup 和 attempt read model 都可从 DB 重建；trace 只加速 UI，不能是唯一唤醒来源。 |
| P2-I12 | attack policy snapshot、fuel consumption 和 residual risks 持久化；达到 cap 时 terminal disclosure，不用模型 prose 冒充收敛。 |

---

## 1. 文件结构

### 1.1 新建

- `backend/crates/golish-db/migrations/20260712000004_attack_execution_v2.sql`：P2 additive schema、legacy partial index 修复、retention FK、composite ownership、evidence ownership trigger、operation contract；`00002` 已由 corrected P1 cutover 使用，`00003` 由 trusted operator principal 使用。
- `backend/crates/golish-db/migrations/20260712000005_attack_execution_v2_cutover.sql`：仅 Task 12 shadow/e2e gate 全绿后创建；顺序推进 attack rollout，旧 operation contract 不变。
- `backend/crates/golish-db/tests/attack_execution_v2_migrations.rs`：embedded PostgreSQL migration、hostile sibling/retention/ownership fixtures。
- `backend/crates/golish-db/src/repo/attack_candidate_seeds.rs`：formulaic observation seed 与 seed evidence。
- `backend/crates/golish-db/src/repo/attack_execution_rollout.rs`：monotonic deployment default CAS；operation 创建时冻结到 row。
- `backend/crates/golish-db/src/repo/attack_candidate_work_items.rs`：reasoning manifest、Candidate/NoCandidateDecision terminalization。
- `backend/crates/golish-db/src/repo/attack_candidate_approvals.rs`：plan-bound approval versions、expiry/revoke。
- `backend/crates/golish-db/src/repo/candidate_attempts.rs`：Attempt result state、action journal；不拥有 lease/checkpoint。
- `backend/crates/golish-db/src/repo/attack_waves.rs`：operation wave、per-org unit、review barrier/wakeup 和 policy snapshot。
- `backend/crates/golish-db/src/repo/attack_execution_lanes.rs`：global exploit lane，与 P1 WorkerRun compound claim/heartbeat/release。
- `backend/crates/golish-db/src/repo/attack_fact_deltas.rs`：FactDelta、evidence joins、consume transaction 和 residual risks。
- `backend/crates/golish-db/src/repo/finding_lineage.rs`：terminalizer 的 Finding + lineage 单事务入口。
- `backend/crates/golish-agent-kit/src/harness/attack_execution/mod.rs`：P2 纯领域模块入口。
- `backend/crates/golish-agent-kit/src/harness/attack_execution/types.rs`：稳定 plan/result/review/wave DTO。
- `backend/crates/golish-agent-kit/src/harness/attack_execution/state.rs`：Candidate/Approval/Attempt/Wave 纯状态机。
- `backend/crates/golish-agent-kit/src/harness/attack_execution/classifier.rs`：versioned immutable CandidateExecutionPlan classifier。
- `backend/crates/golish-agent-kit/src/harness/attack_execution/review_barrier.rs`：DB snapshot 到 interrupt/branch 的纯决策。
- `backend/crates/golish-agent-kit/src/harness/attack_execution/verification_gate.rs`：exact wave/unit terminal truth validator。
- `backend/crates/golish-agent-kit/src/harness/attack_execution/fact_delta.rs`：delta validation、dedupe 和 fuel decision。
- `backend/crates/golish-agent-kit/src/harness/attack_execution/tests.rs`：纯状态机、classifier、barrier、gate、fuel tests。
- `backend/crates/golish-core/src/attack_execution.rs`：跨 DB/runtime 可复用的 rollout enum 与 opaque CandidateAttempt identity；不含 capability/action 类型。
- `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/candidate_verification.rs`：逐条 scheduler、foreground verifier、submit validator 调度。
- `backend/crates/golish-agent-app/src/ai/db_bridge/attack_execution.rs`：`DbRepoProvider` 的 attack execution bridge。
- `backend/crates/golish-agent-app/src/ai/commands/attack.rs`：list/review/resume/attempt Tauri commands 与 ts-rs DTO。
- `backend/crates/golish-agent-app/src/ai/commands/core/operation_resume.rs`：聊天 resume 与 candidate review wakeup 共用的 trusted operation-resume service。
- `backend/crates/golish-pentest-domain/src/finding_write.rs`：非模型可见的 Finding write context/authority primitives。
- `backend/crates/golish-pentest-app/src/pentest_bridge/verification_capabilities.rs`：只接收 action ordinal、从 trusted Attempt context 重载 exact plan、强制前台执行的 verifier wrapper。
- `frontend/lib/api/attack.ts`：唯一 Tauri IPC wrapper。
- `frontend/components/Engagement/AttackCandidateReview.tsx`、`frontend/components/Engagement/AttackCandidateReview.test.tsx`：loading/error/empty/review/resume UI。
- `frontend/components/Engagement/CandidateAttemptRows.tsx`、`frontend/components/Engagement/CandidateAttemptRows.test.tsx`：Attempt/evidence/lineage/residual read model。
- `frontend/lib/generated/{AttackCandidateReviewItem.ts,AttackCandidateReviewState.ts,AttackCandidateReviewRequest.ts,AttackCandidateReviewResponse.ts,CandidateAttemptRow.ts}`：只由 `just gen-types` 生成，禁止手写。

### 1.2 修改

- `backend/crates/golish-db/src/repo/{mod.rs,operation_state.rs,attack_candidates.rs,findings.rs}`。
- `backend/crates/golish-agent-kit/src/{db_traits/mod.rs,db_traits/types.rs,harness/mod.rs,harness/types.rs,harness/stage_capability.rs,harness/pre_action_authorizer.rs,harness/gate/rule_engine.rs,harness/phase_flow.rs,harness/operation_flow.rs,task_orchestrator/subtask_phases/execute.rs}`。
- `backend/crates/golish-agent-app/src/ai/{db_bridge/mod.rs,harness_submit_tool.rs,commands/mod.rs,commands/core/mod.rs,commands/core/chat.rs}`。
- `backend/crates/golish-agent-runtime/src/agentic_loop/{context.rs,tool_execution/direct/mod.rs,turn/phases/tool_dispatch.rs}`。
- `backend/crates/golish-core/src/{lib.rs,agent_session.rs,events/harness_trace.rs}`。
- `backend/crates/golish-app-core/src/{background_jobs.rs,pty_interactive.rs}`：增加带错误返回的 candidate-context background guard，不把 process registry 提升为 durable truth。
- `backend/crates/golish-sub-agents/src/{executor_types.rs,defaults/builder/mod.rs,defaults/builder/registry.rs,defaults/prompts/orchestration.rs,defaults/tests.rs}`。
- `backend/crates/golish-pentest-app/src/pentest_bridge/{mod.rs,record_finding.rs,auth_probe.rs,vuln_capabilities.rs,verification_capabilities.rs}`。
- `backend/crates/golish-pentest/src/output_store/findings.rs`。
- `backend/crates/golish-recon-app/src/sensitive_scan.rs`。
- `backend/crates/golish-scan-runner/src/{feroxbuster.rs,nuclei/runner.rs}`。
- `backend/crates/golish/src/{commands_facade/mod.rs,commands_registry.rs}` 与新 `backend/crates/golish/src/commands_facade/attack.rs`。
- `frontend/lib/api/{index.ts,error-codes.ts}`；`frontend/lib/generated/` 只由 ts-rs 测试生成，禁止手写。
- `frontend/components/ToolCallDetailView/ToolCallDetailView.tsx`。
- `frontend/services/ai-events/{harness-handlers.ts,harness-handlers.test.ts}`。
- `frontend/store/{stage-run.test.ts,slices/session-core.ts,types/session.ts}`。
- `resources/harness/stages/{vuln_triage,attack_candidate,verification}/{spec.json,methodology.md}`、`resources/harness/graph/phases.json`。
- `scripts/check_repo_ownership.py`、`scripts/run_tree.py`。
- `docs/modules/INDEX.md` 以及本计划触及模块对应卡片。
- `agent-progress.md`、`feature_list.json`。

---

## Task 0：锁定 P1 handoff、rollout contract 与脏树边界

**文件：** 只读 `docs/superpowers/plans/2026-07-12-runtime-memory-foundation-corrected.md`、`feature_list.json`、`agent-progress.md`、`backend/crates/golish-agent-kit/src/harness/runtime_memory_contract.rs`；新建 `backend/crates/golish-core/src/attack_execution.rs`；修改 `backend/crates/golish-core/src/lib.rs`、`backend/crates/golish-db/src/repo/operation_state.rs`。

### 步骤 1：记录仓库与 P1 前置证据

执行：

```bash
git status --short --branch
rg -n 'runtime-memory-candidate-pipeline-v2-2026-07-12|in_progress' feature_list.json agent-progress.md
rg -n 'RuntimeMemoryContract|runtime_memory_contract|stage_deliverable_submissions|work_item_kind' backend/crates/golish-agent-kit backend/crates/golish-db
```

预期：V2 是唯一 `in_progress`；P1 的 corrected contract、trusted submission identity 和 WorkerRun work-item identity 均存在。若任一项不存在，停止 P2 并完成 P1；不在 P2 里复制 P1 类型。

### 步骤 2：写 operation contract RED

在 `backend/crates/golish-db/src/repo/operation_state.rs` 的 tests 加；runtime contract 使用 DB row 的稳定 snake-case 值，避免让 `golish-db` 反向依赖 `golish-agent-kit`：

```rust
#[test]
fn attack_contract_cannot_enable_v2_on_non_v2_runtime_memory() {
    assert!(validate_operation_contracts(
        "dual_write_read_v2_fallback",
        AttackExecutionContract::V2Only,
    )
    .is_err());
    assert!(validate_operation_contracts(
        "v2_only",
        AttackExecutionContract::V2Only,
    )
    .is_ok());
}
```

运行并确认因 `AttackExecutionContract`/validator 不存在而 RED：

```bash
cd backend && cargo nextest run -p golish-db attack_contract_cannot_enable_v2 --no-tests=fail --status-level fail
```

### 步骤 3：实现最小 contract 类型与纯 validator

把 `AttackExecutionContract` 放在 `golish-core/src/attack_execution.rs` 并从 `golish-core/src/lib.rs` re-export；实现纯 validator，但本任务不让 SQL 引用尚未迁移的列。migration 的 rollout singleton、DB constraint 和 immutable trigger 在 Task 2 落地；`operation_state::insert` 的同事务 freeze 在 Task 3 落地。禁止从环境变量覆盖已存在行。

### 步骤 4：GREEN 与 package gate

```bash
cd backend && cargo nextest run -p golish-db attack_contract --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-core attack_execution --no-tests=fail --status-level fail
cd backend && cargo clippy -p golish-db --all-targets -- -D warnings
cd backend && cargo clippy -p golish-core --all-targets -- -D warnings
cargo fmt --all -- --check
```

### 步骤 5：提交

```bash
git add -- backend/crates/golish-core/src/attack_execution.rs backend/crates/golish-core/src/lib.rs backend/crates/golish-db/src/repo/operation_state.rs agent-progress.md
git diff --cached --check
git commit -m "feat(attack): define per-operation execution contract"
```

---

## Task 1：先定义纯领域类型、完整终态与 immutable classifier

**文件：** 新建 `backend/crates/golish-agent-kit/src/harness/attack_execution/{mod.rs,types.rs,state.rs,classifier.rs,tests.rs}`；修改 `backend/crates/golish-agent-kit/src/harness/{mod.rs,stage_capability.rs}`。

### 步骤 1：写状态机与结果 DTO RED

先写这些具体测试：

```rust
#[test]
fn verified_requires_proof_and_finding_draft() {
    let result = CandidateAttemptResult::verified_without_proof_fixture();
    assert_eq!(validate_terminal_result(&result).unwrap_err().code(), "ATTACK_PROOF_REQUIRED");
}

#[test]
fn refuted_requires_refutation_evidence_and_no_finding() {
    let result = CandidateAttemptResult::refuted_with_finding_fixture();
    assert_eq!(validate_terminal_result(&result).unwrap_err().code(), "ATTACK_REFUTED_FINDING_FORBIDDEN");
}

#[test]
fn blocked_requires_stable_reason_or_blocker_evidence() {
    let result = CandidateAttemptResult::blocked_without_reason_fixture();
    assert_eq!(validate_terminal_result(&result).unwrap_err().code(), "ATTACK_BLOCK_REASON_REQUIRED");
}

#[test]
fn attempt_has_no_waiting_background_transition() {
    assert!(transition_attempt(AttemptStatus::Running, AttemptEvent::Backgrounded).is_err());
}
```

运行：

```bash
cd backend && cargo nextest run -p golish-agent-kit attack_execution --no-tests=fail --status-level fail
```

预期：模块和类型尚不存在，RED。

### 步骤 2：定义完整稳定类型

`types.rs` 必须一次定义并在后续任务保持同名：

```rust
pub const CANDIDATE_PLAN_SCHEMA_V1: &str = "candidate-plan-v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CandidateExecutionPlan {
    pub schema_version: String,
    pub classifier_version: String,
    pub candidate_id: Uuid,
    pub target_identity_hash: String,
    pub actions: Vec<PlannedCandidateAction>,
    pub budget: CandidateBudget,
    pub foreground_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlannedCandidateAction {
    pub ordinal: u32,
    pub capability_id: String,
    pub action_kind: String,
    pub canonical_args: serde_json::Value,
    pub side_effect_class: SideEffectClass,
    pub required_evidence_role: AttemptEvidenceRole,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CandidateAttemptResult {
    pub attempt_id: Uuid,
    pub candidate_plan_hash: String,
    pub disposition: AttemptDisposition,
    pub proof_evidence_ids: Vec<i64>,
    pub refutation_evidence_ids: Vec<i64>,
    pub blocker_evidence_ids: Vec<i64>,
    pub blocker_reason_code: Option<String>,
    pub finding: Option<VerifiedFindingDraft>,
    pub fact_deltas: Vec<FactDeltaDraft>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerifiedFindingDraft {
    pub title: String,
    pub severity: FindingSeverity,
    pub cvss: Option<f64>,
    pub affected_target: String,
    pub description: String,
    pub reproduction_steps: Vec<String>,
    pub remediation: String,
}
```

Attempt 状态固定为 `queued|running|submitted|verified|refuted|blocked|retryable_failed|abandoned`。不定义 `leased`、`waiting_background`、lease token 或 checkpoint；运行 ownership 来自 P1 WorkerRun。

### 步骤 3：写 classifier RED

```rust
#[test]
fn classifier_is_canonical_and_foreground_only() {
    let a = classify_candidate(&candidate_fixture()).unwrap();
    let b = classify_candidate(&candidate_fixture_with_reordered_prior_refs()).unwrap();
    assert_eq!(canonical_plan_hash(&a).unwrap(), canonical_plan_hash(&b).unwrap());
    assert!(a.foreground_only);
    assert!(a.actions.iter().all(|action| !action.canonical_args["background"].as_bool().unwrap_or(false)));
}

#[test]
fn unsupported_technique_fails_closed_before_review() {
    let err = classify_candidate(&unsupported_candidate_fixture()).unwrap_err();
    assert_eq!(err.code(), "ATTACK_CAPABILITY_UNSUPPORTED");
}
```

### 步骤 4：实现 versioned registry

`classifier.rs` 用静态、可审查 registry 把 technique/risk/target class 映射到 backend-owned capability recipe。canonical hash 使用递归 key-sort 后的 JSON bytes + SHA-256；不得直接 hash 模型原始 JSON。`stage_capability.rs` 同步把：

- `vuln.run_formulaic_sweep.writes` 改为 `audit_log`、`technique_outcomes`、`attack_candidate_seeds`。
- `attack.synthesize_candidates.writes` 改为 `attack_candidate_work_items`、`attack_candidates`，并注明只有 final Gate PASS transaction 可写 Candidate。
- `verify.validate_candidate` 的 raw `sqlmap|metasploit|searchsploit` 列表改成 classifier registry 中真实 backend capability ids；write set 只列 Attempt evidence/result、Finding lineage、FactDelta。

### 步骤 5：GREEN、clippy、提交

```bash
cd backend && cargo nextest run -p golish-agent-kit attack_execution --no-tests=fail --status-level fail
cd backend && cargo clippy -p golish-agent-kit --all-targets -- -D warnings
cargo fmt --all -- --check
git add -- backend/crates/golish-agent-kit/src/harness/attack_execution backend/crates/golish-agent-kit/src/harness/mod.rs backend/crates/golish-agent-kit/src/harness/stage_capability.rs
git diff --cached --check
git commit -m "feat(attack): define candidate execution domain contract"
```

---

## Task 2：新增修正版 schema，先把 index、retention、ownership、evidence 和 policy 做对

**文件：** 新建 `backend/crates/golish-db/migrations/20260712000004_attack_execution_v2.sql`、`backend/crates/golish-db/tests/attack_execution_v2_migrations.rs`。

### 步骤 1：写 embedded migration RED

测试名必须覆盖：

```rust
#[tokio::test]
#[serial_test::serial]
async fn v2_same_candidate_hash_is_isolated_by_frozen_org() {}

#[tokio::test]
#[serial_test::serial]
async fn deleting_live_org_and_target_retains_attack_audit_rows_and_nulls_live_target_ref() {}

#[tokio::test]
#[serial_test::serial]
async fn hostile_sibling_approval_attempt_and_lineage_inserts_fail_in_db() {}

#[tokio::test]
#[serial_test::serial]
async fn foreign_or_non_evidence_audit_id_cannot_be_linked() {}

#[tokio::test]
#[serial_test::serial]
async fn v2_contract_requires_runtime_memory_v2_and_is_immutable() {}
```

测试先跑完整 migration，再 seed 两个 sibling org、两个 scope units、两个 wave units、两个 targets 和 operation-scoped evidence。运行并确认缺表/约束导致 RED：

这里的删除 fixture 直接执行满足 invalidation 前置后的 DB delete，用于证明 P2 retained rows 的 FK 行为；它不绕过 corrected P1 当前对 `organization_delete` command 的 `runtime_scope_history_requires_invalidation` 应用层阻断。

```bash
cd backend && cargo nextest run -p golish-db --test attack_execution_v2_migrations --no-tests=fail --status-level fail
```

### 步骤 2：修复 operation contract 与 legacy unique index

DDL 必须包含：

```sql
CREATE TABLE attack_execution_rollout (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    contract TEXT NOT NULL CHECK (contract IN (
        'legacy','dual_write_read_legacy','dual_write_read_v2_fallback','v2_only'
    )),
    rank SMALLINT NOT NULL CHECK (rank BETWEEN 0 AND 3),
    row_version BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
INSERT INTO attack_execution_rollout(singleton, contract, rank)
VALUES (TRUE, 'legacy', 0);

ALTER TABLE operation_state
    ADD COLUMN attack_execution_contract TEXT NOT NULL DEFAULT 'legacy'
    CHECK (attack_execution_contract IN ('legacy','dual_write_read_legacy','dual_write_read_v2_fallback','v2_only'));

ALTER TABLE operation_state
    ADD CONSTRAINT operation_v2_attack_requires_v2_runtime
    CHECK (attack_execution_contract <> 'v2_only' OR runtime_memory_contract = 'v2_only');

CREATE FUNCTION reject_operation_attack_contract_change()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.attack_execution_contract IS DISTINCT FROM OLD.attack_execution_contract THEN
        RAISE EXCEPTION 'operation attack execution contract is immutable';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER operation_attack_contract_immutable
BEFORE UPDATE OF attack_execution_contract ON operation_state
FOR EACH ROW EXECUTE FUNCTION reject_operation_attack_contract_change();

ALTER TABLE attack_candidates
    ADD COLUMN operation_uuid UUID REFERENCES operation_state(operation_id) ON DELETE RESTRICT,
    ADD COLUMN target_identity_hash TEXT;

DROP INDEX IF EXISTS uq_attack_candidates_op_target_hash;
CREATE UNIQUE INDEX uq_attack_candidates_legacy_op_target_hash
    ON attack_candidates(operation_id, target, hypothesis_hash)
    WHERE operation_uuid IS NULL;

CREATE UNIQUE INDEX uq_attack_candidates_v2_identity
    ON attack_candidates(operation_uuid, organization_id, target_identity_hash, hypothesis_hash)
    WHERE operation_uuid IS NOT NULL;
```

`attack_candidates.rs` 的 legacy UPSERT 在 Task 3 改成：

```sql
ON CONFLICT (operation_id, target, hypothesis_hash)
WHERE operation_uuid IS NULL
DO UPDATE SET updated_at = NOW()
```

不得保留一个无 predicate 的旧三列 unique index，否则 sibling org V2 行仍会相撞。

`attack_execution_rollout` 的 repo/trigger 只允许 `rank + 1` 的 compare-and-swap；拒绝 downgrade、skip 和 stale `row_version`。operation 创建必须在同一 transaction 读取 singleton 并写 `operation_state.attack_execution_contract`。

### 步骤 3：创建 wave、seed、work-item 与 Candidate ownership spine

先移除 legacy Candidate 的 live-org cascade，并建立 nullable live target 引用；冻结 organization UUID 不加 live FK：

```sql
ALTER TABLE attack_candidates
    DROP CONSTRAINT IF EXISTS attack_candidates_organization_id_fkey;
ALTER TABLE attack_candidates
    ADD COLUMN target_live_id UUID REFERENCES targets(id) ON DELETE SET NULL,
    ADD COLUMN target_type_at_time TEXT,
    ADD COLUMN target_value_at_time TEXT;
```

V2 CHECK 要求 `organization_id`、frozen target fields 和 ownership spine 非空；`target_live_id` 与 terminal ids 必须保持 nullable，才能在 live target 删除时 SET NULL 或在未终态时为空。legacy `operation_uuid IS NULL` 行继续允许原有 nullable 字段。

schema 必须包含以下表和稳定 composite key：

| 表 | 必需 ownership 列 | 关键约束 |
|---|---|---|
| `attack_wave_runs` | `id,operation_id,scope_snapshot_id,generation` | `UNIQUE(id,operation_id,scope_snapshot_id)`；scope snapshot/operation composite FK；持久化 `policy_snapshot,policy_hash,max_*` |
| `attack_wave_units` | `id,wave_run_id,operation_id,scope_snapshot_id,organization_id` | wave composite FK；scope membership composite FK；每 frozen org 一行，no-work 也建行 |
| `attack_candidate_seeds` | `id,wave_unit_id,operation_id,scope_snapshot_id,organization_id,target_live_id,target_*_at_time,target_identity_hash` | `target_live_id REFERENCES targets(id) ON DELETE SET NULL`；无 organizations live FK |
| `attack_candidate_work_items` | seed/wave ownership spine + `work_item_key,decision_kind,candidate_id,no_candidate_reason_code,no_candidate_detail` | `UNIQUE(wave_unit_id,work_item_key)`；terminal check 保证 candidate/no_candidate 互斥且 no_candidate 有 reason |
| `attack_candidates` 扩展 | `operation_uuid,scope_snapshot_id,wave_run_id,wave_unit_id,source_work_item_id,source_stage_run_unit_id,source_deliverable_submission_id,target_live_id,target_type_at_time,target_value_at_time,target_identity_hash,execution_plan,candidate_plan_hash,risk_class,row_version,terminal_attempt_id,terminal_finding_id` | V2 ownership/frozen/plan fields non-null；`target_live_id`/terminal ids nullable；live target SET NULL；组织 UUID 是 frozen identity |

`attack_wave_units` 必须引用 P1 trusted `stage_run_unit_id` 和 `deliverable_submission_id`，不得从 tool-call 时间、session id 或 deliverable wire UUID 猜 execution identity。

### 步骤 4：创建 Approval、Attempt、action journal、barrier 和 lane

- `attack_candidate_approvals` 重复 candidate ownership spine 与 `target_identity_hash,candidate_plan_hash`，用 composite FK 指向 Candidate；保存 exact `execution_plan,allowed_capability_ids,allowed_action_kinds,budget,expires_at,decision_version,status,decided_by`。
- `candidate_attempts` 重复 ownership spine，composite FK 指向 exact Approval；保存 `ordinal,status,stage_worker_run_id,result_json,result_hash,terminal_at`。禁止出现 lease/checkpoint/background job 字段。
- `candidate_attempt_actions` 以 `(attempt_id,action_ordinal)` unique，状态 `planned|started|completed|failed|outcome_unknown`；恢复时 `outcome_unknown` 必须 BLOCK/重审，禁止盲重放外部副作用。
- `candidate_review_barriers` 每 wave run 一行，状态 `open|resume_pending|dispatching|resumed|terminal`，保存 `resume_version,last_error,dispatch_started_at`。它是 durable wake source，不依赖 trace。
- `attack_execution_lanes` seed `global:exploit`，只保存 `stage_worker_run_id,lease_token,lease_owner,lease_expires_at`；lane 与 P1 WorkerRun 在 repo compound transaction 中共同更新。

固定 DB 锁序为：`operation_state -> attack_wave_runs -> attack_wave_units -> attack_execution_lanes -> attack_candidates -> attack_candidate_approvals -> candidate_attempts -> stage_worker_runs`。所有 repo tests 必须使用同一顺序。

### 步骤 5：把 evidence array 全部换成 join table

创建：

- `attack_candidate_seed_evidence(seed_id,evidence_id,role)`
- `attack_candidate_work_item_evidence(work_item_id,evidence_id,role)`
- `attack_candidate_evidence(candidate_id,evidence_id,role)`
- `candidate_attempt_evidence(attempt_id,evidence_id,role)`
- `attack_fact_delta_evidence(fact_delta_id,evidence_id,role)`
- `attack_residual_risk_evidence(residual_risk_id,evidence_id,role)`

每张 join table 用 PK `(owner_id,evidence_id,role)`，`evidence_id REFERENCES audit_log(id) ON DELETE RESTRICT`。共用 constraint trigger 在 INSERT/UPDATE 时检查：

```sql
audit_log.audit_role = 'evidence'
AND audit_log.run_id = owner.operation_id
AND (audit_log.detail->>'organization_id')::uuid = owner.organization_id
AND (audit_log.target_id IS NULL OR audit_log.target_id = owner.target_live_id)
```

target 已删除后的历史 join 不重新失败；trigger 校验发生在 link 创建时，冻结 target hash 留存。不得以 Rust 预查代替 DB trigger。

### 步骤 6：创建 Finding lineage、FactDelta 和 residual risk

- `finding_lineage` 对 `candidate_attempt_id` UNIQUE，重复完整 ownership spine、frozen target identity，并 `finding_id REFERENCES findings(id) ON DELETE RESTRICT`。
- `attack_fact_deltas` 保存 `source_attempt_id,operation_id,scope_snapshot_id,organization_id,target_identity_hash,canonical_ref_kind,canonical_ref_id,canonical_ref_version,delta_kind,dedupe_hash,status`，不存 evidence array。
- `attack_residual_risks` 保存触发 cap/revoke/environment blocker 的 stable reason、policy hash、wave/attempt counters、disclosure status；报告层可以确定性读取。

### 步骤 7：GREEN、schema 检查、提交

```bash
cd backend && cargo nextest run -p golish-db --test attack_execution_v2_migrations --no-tests=fail --status-level fail
cd backend && cargo clippy -p golish-db --all-targets -- -D warnings
cargo fmt --all -- --check
git diff --check -- backend/crates/golish-db/migrations/20260712000004_attack_execution_v2.sql backend/crates/golish-db/tests/attack_execution_v2_migrations.rs
git add -- backend/crates/golish-db/migrations/20260712000004_attack_execution_v2.sql backend/crates/golish-db/tests/attack_execution_v2_migrations.rs
git commit -m "feat(db): add constrained attack execution v2 schema"
```

---

## Task 3：实现 scoped repos 与 compound transactions

**文件：** 新建 `backend/crates/golish-db/src/repo/{attack_execution_rollout.rs,attack_candidate_seeds.rs,attack_candidate_work_items.rs,attack_candidate_approvals.rs,candidate_attempts.rs,attack_waves.rs,attack_execution_lanes.rs,attack_fact_deltas.rs,finding_lineage.rs}`；修改 `backend/crates/golish-db/src/repo/{mod.rs,attack_candidates.rs,operation_state.rs,findings.rs}`。

### 步骤 1：写 repo RED

在各 repo 内 tests 和 migration integration test 增加：

```rust
#[tokio::test]
async fn accept_candidate_batch_requires_final_pass_and_complete_manifest() {}

#[tokio::test]
async fn review_batch_is_plan_bound_org_scoped_and_reopens_after_expiry() {}

#[tokio::test]
async fn compound_claim_owns_worker_and_lane_with_one_lease_token() {}

#[tokio::test]
async fn heartbeat_and_release_update_worker_and_lane_atomically() {}

#[tokio::test]
async fn terminalize_verified_is_idempotent_and_rejects_foreign_proof() {}
```

运行：

```bash
cd backend && cargo nextest run -p golish-db -E 'test(attack_candidate) | test(candidate_attempt) | test(attack_wave) | test(attack_execution_lane) | test(finding_lineage)' --no-tests=fail --status-level fail
```

### 步骤 2：实现明确 repo API

签名固定为：

```rust
pub async fn seed_wave_work_items(
    tx: &mut Transaction<'_, Postgres>,
    command: SeedAttackWorkItems,
) -> Result<SeedAttackWorkItemsResult>;

pub async fn advance_attack_execution_rollout(
    tx: &mut Transaction<'_, Postgres>,
    expected_version: i64,
    next: AttackExecutionContract,
) -> Result<AttackExecutionRolloutRow>;

pub async fn accept_gate_passed_candidate_batch(
    tx: &mut Transaction<'_, Postgres>,
    command: AcceptCandidateBatch,
) -> Result<AcceptedCandidateBatch>;

pub async fn review_wave_candidates(
    tx: &mut Transaction<'_, Postgres>,
    command: ReviewCandidateBatch,
) -> Result<WaveReviewResult>;

pub async fn claim_next_candidate_attempt(
    pool: &PgPool,
    query: CandidateClaimQuery,
) -> Result<Option<ClaimedCandidateAttempt>>;

pub async fn heartbeat_candidate_execution(
    pool: &PgPool,
    heartbeat: CandidateExecutionHeartbeat,
) -> Result<HeartbeatOutcome>;

pub async fn record_attempt_submission(
    tx: &mut Transaction<'_, Postgres>,
    command: RecordAttemptSubmission,
) -> Result<RecordedAttemptSubmission>;

pub async fn terminalize_verified_finding(
    tx: &mut Transaction<'_, Postgres>,
    command: TerminalizeVerifiedFinding,
) -> Result<TerminalizedFinding>;
```

`claim_next_candidate_attempt` 必须：检查两个 operation contract；锁 lane；`FOR UPDATE SKIP LOCKED` 选 exact approved candidate；验证 current approval/plan hash/expiry；创建或恢复同 Attempt；创建或读取 `work_item_kind='candidate_attempt', work_item_key=attempt_id` 的 P1 WorkerRun；用同一 lease token claim WorkerRun 与 lane；事务提交后才返回 dispatch payload。

`operation_state::insert` 在本任务改为：同一短事务锁 `attack_execution_rollout` singleton、读取 contract、验证 runtime/attack 组合、插入 operation row；已存在 operation 永远不重读 singleton。

### 步骤 3：修 legacy UPSERT 与 retained target reads

`attack_candidates.rs` 拆为 explicit `upsert_legacy_by_hash` 和 `accept_v2_batch_in_transaction`。legacy SQL 使用 Task 2 的 partial conflict predicate；V2 不调用 legacy upsert。读 DTO 同时返回 `target_live_id` 和 frozen target identity，UI 不因为 live target 已删除而丢行。

### 步骤 4：实现 review expiry/reopen

approval 在 Attempt 开始前过期/撤销时：current approval 置 `expired|revoked`，Candidate 回 `proposed`，wave unit `review_closed=false`，barrier 回 `open`。Attempt 已 running 时不回 proposed：authorizer 阻止下一 action，Attempt terminalize `blocked`，写稳定 reason/residual risk，release WorkerRun+lane；新执行必须新 approval + 新 ordinal Attempt。

### 步骤 5：GREEN 与 package gate

```bash
cd backend && cargo nextest run -p golish-db -E 'test(attack_candidate) | test(candidate_attempt) | test(attack_wave) | test(attack_execution_lane) | test(finding_lineage)' --no-tests=fail --status-level fail
cd backend && cargo clippy -p golish-db --all-targets -- -D warnings
cargo fmt --all -- --check
```

### 步骤 6：提交

```bash
git add -- backend/crates/golish-db/src/repo/mod.rs backend/crates/golish-db/src/repo/operation_state.rs backend/crates/golish-db/src/repo/attack_candidates.rs backend/crates/golish-db/src/repo/findings.rs backend/crates/golish-db/src/repo/attack_execution_rollout.rs backend/crates/golish-db/src/repo/attack_candidate_seeds.rs backend/crates/golish-db/src/repo/attack_candidate_work_items.rs backend/crates/golish-db/src/repo/attack_candidate_approvals.rs backend/crates/golish-db/src/repo/candidate_attempts.rs backend/crates/golish-db/src/repo/attack_waves.rs backend/crates/golish-db/src/repo/attack_execution_lanes.rs backend/crates/golish-db/src/repo/attack_fact_deltas.rs backend/crates/golish-db/src/repo/finding_lineage.rs
git diff --cached --check
git commit -m "feat(db): add attack queue compound transactions"
```

---

## Task 4：扩展 DbRepoProvider，并把 scanner hit 限定为 observation seed

**文件：** 修改 `backend/crates/golish-agent-kit/src/db_traits/{mod.rs,types.rs}`、`backend/crates/golish-agent-app/src/ai/db_bridge/{mod.rs,attack_execution.rs,evidence.rs}`、`backend/crates/golish-pentest-app/src/pentest_bridge/vuln_capabilities.rs`、`backend/crates/golish-agent-kit/src/harness/stage_capability.rs`。

### 步骤 1：写 bridge 与 seed RED

```rust
#[tokio::test]
async fn formulaic_hit_writes_seed_and_evidence_but_not_candidate_or_finding() {
    let repo = RecordingRepo::default();
    run_formulaic_fixture(&repo).await.unwrap();
    assert_eq!(repo.seed_writes(), 1);
    assert_eq!(repo.candidate_writes(), 0);
    assert_eq!(repo.finding_writes(), 0);
}
```

并为 `DbRepoProvider` mock 写 fail-closed test：P2 method 未实现时返回 `ATTACK_V2_REPO_UNAVAILABLE`，不能默认空集合从而 PASS。

### 步骤 2：新增 narrow DTO/methods

`db_traits/types.rs` 只放序列化/传输所需 DTO；agent-kit 不 import golish-db row。trait methods 覆盖 seed work items、accept batch、review snapshot、claim/heartbeat/release、attempt evidence/result、terminalize、FactDelta consume、barrier wakeup。V2 read methods 默认实现必须 `Err`，不是 `Ok(vec![])`。

### 步骤 3：formulaic wrapper 写 seed

`vuln_capabilities.rs` 在 `attack_execution_contract.writes_v2()` 时把成功命中写入 V2 transaction：

1. canonical observation/evidence；
2. `technique_outcomes`；
3. `attack_candidate_seeds` + seed evidence link。

V2 branch 不构造 `AttackCandidate`、不调用 Candidate state transition、不写 Finding。scanner 空结果同样有 checked-empty evidence，供 reasoning work-item 的 no-candidate 决策引用。`legacy`/dual-read-legacy 的兼容 writer 仍按 contract matrix 存在，但 Task 8 authority 明确 trace 它；V2Only 不可进入该旁路。

### 步骤 4：GREEN、提交

```bash
cd backend && cargo nextest run -p golish-agent-kit -E 'test(attack_execution) | test(db_traits)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-app -E 'test(attack_execution) | test(formulaic)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-pentest-app formulaic --no-tests=fail --status-level fail
cd backend && cargo clippy -p golish-agent-kit -p golish-agent-app -p golish-pentest-app --all-targets -- -D warnings
cargo fmt --all -- --check
git add -- backend/crates/golish-agent-kit/src/db_traits backend/crates/golish-agent-app/src/ai/db_bridge backend/crates/golish-pentest-app/src/pentest_bridge/vuln_capabilities.rs backend/crates/golish-agent-kit/src/harness/stage_capability.rs
git commit -m "feat(attack): bridge formulaic observations into seed queue"
```

---

## Task 5：实现完整 reasoning manifest 与 final Gate PASS Candidate acceptance

**文件：** 修改 `backend/crates/golish-agent-kit/src/harness/{types.rs,gate/rule_engine.rs}`、`backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs`、`backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`、`resources/harness/stages/attack_candidate/{spec.json,methodology.md}`。

### 步骤 1：写 no-candidate 与 pre-gate RED

```rust
#[tokio::test]
async fn empty_candidate_array_with_pending_work_item_blocks() {}

#[tokio::test]
async fn every_work_item_must_end_as_candidate_or_evidenced_no_candidate() {}

#[tokio::test]
async fn gate_block_never_accepts_candidate_rows() {}

#[tokio::test]
async fn final_pass_accepts_once_using_trusted_deliverable_submission_id() {}
```

### 步骤 2：扩展 deliverable draft，而不是信任模型 identity

wire payload 增加：

```rust
pub struct CandidateDecisionDraft {
    pub work_item_key: String,
    pub decision: CandidateDecisionKind,
    pub hypothesis: Option<String>,
    pub rationale: String,
    pub technique: Option<String>,
    pub evidence_refs: Vec<i64>,
    pub no_candidate_reason_code: Option<String>,
}
```

server 从 active stage context 覆写 operation/scope/org/wave/stage execution/submission identity；模型不能传这些字段。每个 Candidate decision 先由 classifier 生成 immutable plan/hash；unsupported plan 使 Gate BLOCK。

### 步骤 3：seed expected manifest

Attack Candidate stage entry 从 exact StageHandoff、formulaic seeds 和 canonical target/technique snapshot 确定性 seed `attack_candidate_work_items`。work-item key 是 canonical source kind/id/version/hash；不能按数组顺序。没有命中也必须为已检查 surface 建 item，最后由 evidence-backed `no_candidate` 关闭。

### 步骤 4：只在 final PASS transaction 接受

删除/禁止所有 gate preview 前的 Candidate persist。`submit_stage_deliverable` final PASS 后，用 P1 trusted `deliverable_submission_id` 调 `accept_gate_passed_candidate_batch`；事务再次验证 active stage execution/unit、scope snapshot、manifest completeness、evidence joins、plan hash，并幂等接受。Gate BLOCK 只保留 submission/evidence 做调试。

### 步骤 5：GREEN、提交

```bash
cd backend && cargo nextest run -p golish-agent-app -E 'test(candidate) | test(harness_submit)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-kit -E 'test(candidate) | test(no_candidate) | test(gate)' --no-tests=fail --status-level fail
cd backend && cargo clippy -p golish-agent-app -p golish-agent-kit --all-targets -- -D warnings
cargo fmt --all -- --check
git add -- backend/crates/golish-agent-kit/src/harness/types.rs backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs resources/harness/stages/attack_candidate
git commit -m "feat(attack): accept complete candidate decisions after final gate"
```

---

## Task 6：实现 durable review API、barrier、UI 与 DB-backed wakeup

**文件：** 新建 `backend/crates/golish-agent-app/src/ai/commands/attack.rs`、`backend/crates/golish-agent-app/src/ai/commands/core/operation_resume.rs`、`backend/crates/golish/src/commands_facade/attack.rs`、`frontend/lib/api/attack.ts`、`frontend/components/Engagement/AttackCandidateReview.tsx` 及 test；修改 commands/db bridge/flow/UI mount 文件。

### 步骤 1：backend RED

```rust
#[tokio::test]
async fn review_rejects_sibling_candidate_stale_plan_and_expired_budget() {}

#[tokio::test]
async fn review_close_sets_durable_resume_pending_and_survives_process_restart() {}

#[tokio::test]
async fn stale_dispatching_wakeup_reopens_without_reopening_review_decisions() {}

#[test]
fn barrier_branches_only_from_exact_db_snapshot() {}
```

### 步骤 2：实现四个 Tauri commands 与 ts-rs DTO

命令固定为：

```text
attack_list_candidate_reviews
attack_review_candidates
attack_resume_candidate_review
attack_list_candidate_attempts
```

所有请求都带 `operation_id + wave_run_id`，后端从 frozen scope snapshot 验 actor/org ownership。错误统一有 `code`：`ATTACK_CANDIDATE_PLAN_CHANGED`、`ATTACK_REVIEW_SCOPE_MISMATCH`、`ATTACK_APPROVAL_EXPIRED`、`ATTACK_REVIEW_ALREADY_CLOSED`、`ATTACK_RESUME_NOT_READY`。

按 `docs/development.md` 五步接线：command → `commands/attack.rs` re-export → `commands_facade/attack.rs` → `commands_registry.rs` → `frontend/lib/api/attack.ts`。运行 `just gen-types` 生成 bindings，禁止手写 `frontend/lib/generated/`。

### 步骤 3：实现 durable resume service

把 `commands/core/chat.rs` 现有 operation resume 逻辑抽到 `commands/core/operation_resume.rs`。review transaction 只写 `resume_pending`；事务提交后 `attack_resume_candidate_review` CAS 到 `dispatching`，再调用共用 trusted resume service。启动成功标 `resumed`；同步失败回 `resume_pending` 并存 `last_error`；startup reaper 把超时 `dispatching` 重置为 `resume_pending`。事务中禁止调用 agent/LLM/外部工具。

### 步骤 4：frontend RED

```tsx
it("reloads an open review from DB even when no trace was observed", async () => {});
it("submits exact plan hashes then exposes Resume verification", async () => {});
it("keeps decisions visible when resume fails and allows idempotent retry", async () => {});
it("renders loading error empty and deleted-target frozen identity states", async () => {});
```

运行：

```bash
pnpm exec vitest run frontend/components/Engagement/AttackCandidateReview.test.tsx frontend/services/ai-events/harness-handlers.test.ts
```

### 步骤 5：实现 UI mount 与 wake hint

`HarnessTraceKind` 加 `CandidateReviewRequired`/`CandidateReviewResumed`，但 handler 收到 trace 后只触发 API refresh。`ToolCallDetailView.tsx` 在 `attack_candidate` stage detail 挂 `AttackCandidateReview`；组件 mount 必须调用 `attack_list_candidate_reviews`，因此 app 重启/漏 trace 仍能恢复。approve/reject 显示 exact plan hash、actions、budget、expiry；live target 已删时展示 frozen target value 和“live target removed”。

### 步骤 6：GREEN 与 package gates

```bash
cd backend && cargo nextest run -p golish-agent-app attack_review --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-kit review_barrier --no-tests=fail --status-level fail
cd backend && cargo clippy -p golish-agent-app -p golish-agent-kit -p golish --all-targets -- -D warnings
just gen-types
pnpm exec vitest run frontend/components/Engagement/AttackCandidateReview.test.tsx frontend/services/ai-events/harness-handlers.test.ts
just check-fe
```

### 步骤 7：提交

```bash
git add -- backend/crates/golish-agent-app/src/ai/commands/attack.rs backend/crates/golish-agent-app/src/ai/commands/mod.rs backend/crates/golish-agent-app/src/ai/commands/core/operation_resume.rs backend/crates/golish-agent-app/src/ai/commands/core/mod.rs backend/crates/golish-agent-app/src/ai/commands/core/chat.rs backend/crates/golish-agent-app/src/ai/db_bridge/attack_execution.rs backend/crates/golish-agent-app/src/ai/db_bridge/mod.rs backend/crates/golish-agent-kit/src/harness/attack_execution/review_barrier.rs backend/crates/golish-agent-kit/src/harness/phase_flow.rs backend/crates/golish-agent-kit/src/harness/operation_flow.rs backend/crates/golish-db/src/repo/attack_waves.rs backend/crates/golish/src/commands_facade/attack.rs backend/crates/golish/src/commands_facade/mod.rs backend/crates/golish/src/commands_registry.rs backend/crates/golish-core/src/events/harness_trace.rs frontend/lib/api/attack.ts frontend/lib/api/index.ts frontend/lib/api/error-codes.ts frontend/lib/generated/AttackCandidateReviewItem.ts frontend/lib/generated/AttackCandidateReviewState.ts frontend/lib/generated/AttackCandidateReviewRequest.ts frontend/lib/generated/AttackCandidateReviewResponse.ts frontend/lib/generated/CandidateAttemptRow.ts frontend/components/Engagement/AttackCandidateReview.tsx frontend/components/Engagement/AttackCandidateReview.test.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.tsx frontend/services/ai-events/harness-handlers.ts frontend/services/ai-events/harness-handlers.test.ts frontend/store/stage-run.test.ts frontend/store/slices/session-core.ts frontend/store/types/session.ts
git diff --cached --check
git commit -m "feat(attack): add durable candidate review and resume wakeup"
```

---

## Task 7：传播 opaque Attempt identity，强制 foreground，并实现 compound scheduler

**文件：** 修改 `backend/crates/golish-core/src/{attack_execution.rs,agent_session.rs}`、`backend/crates/golish-agent-kit/src/harness/pre_action_authorizer.rs`、`backend/crates/golish-agent-runtime/src/agentic_loop/{context.rs,tool_execution/direct/mod.rs,turn/phases/tool_dispatch.rs}`；新建 `backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/candidate_verification.rs`、`backend/crates/golish-pentest-app/src/pentest_bridge/verification_capabilities.rs`；修改 `backend/crates/golish-pentest-app/src/pentest_bridge/mod.rs`、`backend/crates/golish-app-core/src/{background_jobs.rs,pty_interactive.rs}` 和 sub-agent defaults/executor 文件。

### 步骤 1：写 type-boundary 与 foreground RED

```rust
#[test]
fn core_attempt_context_contains_only_opaque_identity() {
    let ctx = CandidateAttemptContextRef {
        candidate_id: candidate(),
        approval_id: approval(),
        attempt_id: attempt(),
        candidate_plan_hash: "sha256:abc".into(),
    };
    assert_eq!(ctx.attempt_id, attempt());
}

#[tokio::test]
async fn candidate_context_rejects_background_execution() {}

#[tokio::test]
async fn two_processes_claim_only_one_global_exploit_attempt() {}

#[tokio::test]
async fn expired_worker_lease_resumes_same_attempt_and_checkpoint() {}
```

### 步骤 2：扩展低层 context

在 `golish-core/src/attack_execution.rs` 定义 `CandidateAttemptContextRef`，只含 `candidate_id,approval_id,attempt_id,candidate_plan_hash`；把 `Option<CandidateAttemptContextRef>` 放进 `AgentToolContext`。operation/org 与 worker lease fencing 继续复用 corrected P1 已放入 `AgentToolContext` 的 opaque identity。不要 import `CandidateExecutionPlan`、`CapabilityId`、action constraint 或 agent-kit enum。runtime 每次 tool dispatch 用这些 opaque IDs 调 DB reload exact current approval/plan。

### 步骤 3：实现 authorizer

每次 action 前验证：operation contracts、scope hash、Candidate/Approval/Attempt ownership、current approval、plan hash、expiry/revoke、action ordinal、capability/action、canonical args、budget、worker lease token、lane ownership。任一 DB read failure都拒绝。candidate context 禁止：

- `background=true`；
- `wait_for_background_jobs`、`check_job` 作为 verifier control flow；
- 不在 exact plan 的 raw `pentest_run` 参数；
- 模型传入 candidate/attempt/approval identity。

`background_jobs.rs` 新增 `try_spawn_for_session_and_tool(&self, command: &str, workspace: &Path, hard_limit: Duration, session_id: Option<String>, tool_context: Option<AgentToolContext>) -> Result<String, BackgroundJobSpawnError>`；它在收到带 `candidate_attempt` 的 tool context 时返回 `ATTACK_VERIFIER_FOREGROUND_REQUIRED`。`pty_interactive.rs` 的唯一 production attributed spawn caller 改走该 Result API 并把 typed error 传回 tool result；无 candidate context 的 legacy `spawn`/`spawn_for_session` 行为不变。不尝试把现有 process `HashMap` 当 durable scheduler。

### 步骤 4：注册 analyst/verifier specialists

两个 defaults builder 同时注册 `attack_analyst` 与 `candidate_verifier`，tests 保证 registry/hardcoded surface 一致。`verification_capabilities.rs` 注册唯一执行 wrapper `verify_execute_candidate_action`；模型参数只有 `action_ordinal`，wrapper 从 trusted context/DB 重载 canonical args，并显式写 `background=false`。verifier 只拥有该 wrapper、`list_recent_evidence` 和 `submit_candidate_attempt`；明确移除 `record_finding`、formulaic scanner、通用 `pentest_run` 和 background controls。

### 步骤 5：实现 scheduler 与 action journal

`candidate_verification.rs` 循环：compound claim → 创建 opaque context → 运行一个 verifier → foreground action 开始前写 action journal `started` → action 完成写 `completed|failed` → submit validator → terminalizer/gate → compound release → 下一条。heartbeat 同一 transaction 更新 WorkerRun 和 lane；Attempt 本身不 heartbeat。进程崩溃后，lease expiry 恢复同 WorkerRun/Attempt；存在 `started` 且无 terminal outcome 的副作用 action 标 `outcome_unknown` 并 BLOCK/重审，不盲重放。

### 步骤 6：GREEN、提交

```bash
cd backend && cargo nextest run -p golish-core candidate_attempt_context --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-kit pre_action_authorizer --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-runtime candidate_verification --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-sub-agents candidate_verifier --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-app-core foreground --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-pentest-app verification_capabilities --no-tests=fail --status-level fail
cd backend && cargo clippy -p golish-core -p golish-agent-kit -p golish-agent-runtime -p golish-sub-agents -p golish-app-core -p golish-pentest-app --all-targets -- -D warnings
cargo fmt --all -- --check
git add -- backend/crates/golish-core/src/attack_execution.rs backend/crates/golish-core/src/agent_session.rs backend/crates/golish-agent-kit/src/harness/pre_action_authorizer.rs backend/crates/golish-agent-runtime/src/agentic_loop/context.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/mod.rs backend/crates/golish-agent-runtime/src/agentic_loop/tool_execution/direct/candidate_verification.rs backend/crates/golish-agent-runtime/src/agentic_loop/turn/phases/tool_dispatch.rs backend/crates/golish-app-core/src/background_jobs.rs backend/crates/golish-app-core/src/pty_interactive.rs backend/crates/golish-pentest-app/src/pentest_bridge/mod.rs backend/crates/golish-pentest-app/src/pentest_bridge/verification_capabilities.rs backend/crates/golish-sub-agents/src/executor_types.rs backend/crates/golish-sub-agents/src/defaults/builder/mod.rs backend/crates/golish-sub-agents/src/defaults/builder/registry.rs backend/crates/golish-sub-agents/src/defaults/prompts/orchestration.rs backend/crates/golish-sub-agents/src/defaults/tests.rs
git commit -m "feat(attack): schedule foreground candidate attempts with one lease owner"
```

---

## Task 8：先实现 submit validator 和 terminalizer，再关闭所有 Finding writer 旁路

**文件：** 新建 `backend/crates/golish-pentest-domain/src/finding_write.rs`；修改 Finding writer 全路径、`harness_submit_tool.rs`、DB terminalizer repo 与 `scripts/check_repo_ownership.py`。

### 步骤 1：写 validator/terminalizer RED

```rust
#[tokio::test]
async fn verified_submission_requires_exact_attempt_proof_and_finding_draft() {}

#[tokio::test]
async fn refuted_and_blocked_terminalize_without_finding_and_with_correct_evidence_role() {}

#[tokio::test]
async fn terminalizer_replay_returns_same_finding_and_lineage() {}

#[tokio::test]
async fn sibling_attempt_proof_cannot_terminalize_candidate() {}
```

### 步骤 2：实现 `submit_candidate_attempt`

tool args 只接受 Task 1 的 `CandidateAttemptResult` 业务字段；attempt identity 从 `AgentToolContext` 覆写。validator 从 DB 重载 plan/approval/evidence links，校验 terminal result、proof/refutation/blocker role、plan hash、action journal terminality、FactDelta draft canonical refs。成功只写 submitted result；Finding 仍未创建。

### 步骤 3：实现单事务 terminalizer

`terminalize_verified_finding` 在一笔短事务中：锁 Attempt/Candidate；验证 current plan/result/proof；通过 `golish-db/src/repo/findings.rs` 创建或复用 Finding；写 `finding_lineage`；更新 Attempt/Candidate terminal ids/status；写 FactDelta/evidence joins；release WorkerRun+lane；commit。任何一步失败全回滚。

### 步骤 4：统一 Finding write authority

`finding_write.rs` 定义内部 context：

```rust
pub enum FindingWriteContext {
    UserCrud,
    LegacyNonHarness,
    HarnessLegacy,
    VerificationTerminalizer { attempt_id: Uuid },
}
```

它不是 Tauri/Tool args。`attack_execution_contract='v2_only'` 的 harness context 只有 terminalizer variant 可写。以下所有当前旁路都改走 `golish-db::repo::findings` 的 guarded API：

- `backend/crates/golish-pentest-app/src/pentest_bridge/record_finding.rs`
- `backend/crates/golish-pentest-app/src/pentest_bridge/auth_probe.rs`
- `backend/crates/golish-pentest/src/output_store/findings.rs`
- `backend/crates/golish-recon-app/src/sensitive_scan.rs`
- `backend/crates/golish-scan-runner/src/nuclei/runner.rs`
- `backend/crates/golish-scan-runner/src/feroxbuster.rs`
- `backend/crates/golish-pentest-app/src/findings/crud.rs`（保留 `UserCrud`）

`record_finding` 在 V2 scanner/attack/verifier specialist tool surface 移除；`tool_taxonomy.rs` 不再把它当 V2 stage 的政策豁免。

### 步骤 5：加静态 ownership ratchet

`scripts/check_repo_ownership.py` 新增规则：`INSERT INTO findings` 只能出现在 migrations/fixtures 和 `backend/crates/golish-db/src/repo/findings.rs`/`finding_lineage.rs`；其它 production file 命中即失败。先运行确认旧 writers 触发 RED，重构后 GREEN。

### 步骤 6：GREEN、提交

```bash
python3 scripts/check_repo_ownership.py
cd backend && cargo nextest run -p golish-db finding_lineage --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-app candidate_attempt --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-pentest-app -E 'test(record_finding) | test(auth_probe) | test(findings)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-scan-runner -E 'test(nuclei) | test(ferox)' --no-tests=fail --status-level fail
cd backend && cargo clippy -p golish-db -p golish-agent-app -p golish-pentest-app -p golish-pentest -p golish-recon-app -p golish-scan-runner --all-targets -- -D warnings
cargo fmt --all -- --check
git add -- backend/crates/golish-pentest-domain/src/finding_write.rs backend/crates/golish-pentest-domain/src/lib.rs backend/crates/golish-db/src/repo/findings.rs backend/crates/golish-db/src/repo/finding_lineage.rs backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs backend/crates/golish-agent-kit/src/harness/tool_taxonomy.rs backend/crates/golish-pentest-app/src/pentest_bridge/record_finding.rs backend/crates/golish-pentest-app/src/pentest_bridge/auth_probe.rs backend/crates/golish-pentest-app/src/findings/crud.rs backend/crates/golish-pentest-app/src/findings/mod.rs backend/crates/golish-pentest/src/output_store/findings.rs backend/crates/golish-recon-app/src/sensitive_scan.rs backend/crates/golish-scan-runner/src/nuclei/runner.rs backend/crates/golish-scan-runner/src/feroxbuster.rs scripts/check_repo_ownership.py
git commit -m "feat(attack): make verification terminalizer the finding authority"
```

---

## Task 9：把 Verification Gate 改成 exact DB truth

**文件：** 修改 `backend/crates/golish-agent-kit/src/harness/{gate/rule_engine.rs,operation_flow.rs,phase_flow.rs}`、`backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs`、`resources/harness/stages/verification/{spec.json,methodology.md}`、`resources/harness/graph/phases.json`。

### 步骤 1：写 RED matrix

```rust
#[test]
fn verified_without_proof_blocks() {}

#[test]
fn refuted_without_refutation_blocks() {}

#[test]
fn blocked_without_reason_or_blocker_evidence_blocks() {}

#[test]
fn empty_verification_passes_only_for_existing_closed_complete_no_candidate_manifest() {}

#[test]
fn missing_or_foreign_db_snapshot_blocks_instead_of_falling_back_to_deliverable() {}
```

### 步骤 2：定义 snapshot

```rust
pub struct VerificationTruthSnapshot {
    pub operation_id: Uuid,
    pub scope_snapshot_id: Uuid,
    pub wave_run_id: Uuid,
    pub wave_unit_id: Uuid,
    pub organization_id: Uuid,
    pub review_closed: bool,
    pub pending_work_items: u32,
    pub approved_ever: u32,
    pub attempts: Vec<AttemptTerminalTruth>,
    pub residual_risks: Vec<ResidualRiskTruth>,
}
```

repo query 必须按所有 key 读取，并包括“曾 approved、现 terminal”的 Candidate，不能只筛当前 `disposition='approved'`。`None`/DB error 一律 BLOCK。

### 步骤 3：替换 gate/flow

Verification Gate 不读 `StageDeliverable.findings`、summary、memory/KG、`spawned_candidates` 或 process `chain_wave_seen`。零 approved 只有在 exact wave/unit 存在、review closed、work-item pending=0、每 item 都有 evidenced no-candidate 且 approved_ever=0 时 PASS/skip。其它空集合 BLOCK。

### 步骤 4：GREEN、提交

```bash
cd backend && cargo nextest run -p golish-agent-kit verification_gate --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-app verification --no-tests=fail --status-level fail
cd backend && cargo clippy -p golish-agent-kit -p golish-agent-app --all-targets -- -D warnings
cargo fmt --all -- --check
git add -- backend/crates/golish-agent-kit/src/harness/attack_execution/verification_gate.rs backend/crates/golish-agent-kit/src/harness/gate/rule_engine.rs backend/crates/golish-agent-kit/src/harness/operation_flow.rs backend/crates/golish-agent-kit/src/harness/phase_flow.rs backend/crates/golish-agent-app/src/ai/harness_submit_tool.rs resources/harness/stages/verification/spec.json resources/harness/stages/verification/methodology.md resources/harness/graph/phases.json
git commit -m "feat(attack): gate verification from exact database truth"
```

---

## Task 10：实现 FactDelta consolidation、fuel 与 residual disclosure

**文件：** 修改 `attack_execution/fact_delta.rs`、`attack_waves.rs`、`attack_fact_deltas.rs`、`operation_flow.rs`、`phase_flow.rs`、`scripts/run_tree.py`。

### 步骤 1：写 RED

```rust
#[tokio::test]
async fn only_unconsumed_evidence_backed_delta_opens_next_wave() {}

#[tokio::test]
async fn sibling_or_stale_canonical_ref_delta_is_rejected() {}

#[tokio::test]
async fn all_org_units_must_be_terminal_before_global_cursor_advances() {}

#[tokio::test]
async fn fuel_cap_closes_wave_and_persists_reportable_residual_risk() {}
```

### 步骤 2：验证 canonical delta

每个 delta 必须：source Attempt terminal；evidence join role=`fact_delta` 且属于同 Attempt；canonical ref 存在、version/hash 匹配 frozen org/scope；dedupe hash 未消费。禁止只提交自由 prose 或 evidence id array。

### 步骤 3：原子 consolidate/open/consume

operation-level transaction 锁当前 wave；确认所有 frozen org unit 的 review/verification/consolidation terminal；比较持久化 policy snapshot 的 `max_waves,max_candidates_total,max_chain_depth,max_attempts_total`；允许时插下一 wave run+每 org unit并 consume accepted deltas；超过 cap 时不打开 wave，写 `attack_residual_risks` 并 terminalize cursor。global cursor 只有该事务可以推进。

### 步骤 4：扩展诊断

`run_tree.py --full --db` 输出 contract、wave policy/hash、per-org review/work-item counts、Attempt/WorkerRun/lane ownership、FactDelta consumed 状态和 residual risk。输出不包含 secret/raw exploit payload。

### 步骤 5：GREEN、提交

```bash
cd backend && cargo nextest run -p golish-db -E 'test(fact_delta) | test(attack_wave) | test(residual)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-kit -E 'test(fact_delta) | test(chain_wave)' --no-tests=fail --status-level fail
python3 -m py_compile scripts/run_tree.py
cd backend && cargo clippy -p golish-db -p golish-agent-kit --all-targets -- -D warnings
cargo fmt --all -- --check
git add -- backend/crates/golish-db/src/repo/attack_fact_deltas.rs backend/crates/golish-db/src/repo/attack_waves.rs backend/crates/golish-agent-kit/src/harness/attack_execution/fact_delta.rs backend/crates/golish-agent-kit/src/harness/operation_flow.rs backend/crates/golish-agent-kit/src/harness/phase_flow.rs scripts/run_tree.py
git commit -m "feat(attack): consolidate evidence-backed attack waves"
```

---

## Task 11：完成 Attempt UI、trace、stage contracts 与 package integration

**文件：** 新建 `CandidateAttemptRows` 及 test；修改 Task 6 UI/handler/store、stage specs/methodologies、sub-agent prompts/defaults、模块卡。

### 步骤 1：frontend RED

```tsx
it("renders attempt ordinal status exact evidence roles and finding lineage", async () => {});
it("shows blocked residual risk without presenting it as verified", async () => {});
it("shows one active exploit lane and a queued next candidate", async () => {});
it("reloads attempts from API after a missed terminal trace", async () => {});
```

### 步骤 2：实现 read model

`CandidateAttemptRows` 通过 `attack_list_candidate_attempts` 获取 DB truth；trace 只触发 refresh。loading/error/empty 三态齐全。verified 显示 Finding link + proof；refuted 显示 refutation；blocked 显示 reason/residual；不得把 Candidate hypothesis 渲染成 Finding。

### 步骤 3：统一 stage contracts

资源必须表达：

```text
vuln_triage -> observation seeds only
attack_candidate -> complete work-item manifest -> final Gate PASS -> candidate review
candidate review -> durable resume
verification -> one foreground CandidateAttempt -> submit -> terminalizer
FactDelta -> optional next attack wave
no delta or fuel cap -> reporting with residual disclosure
```

在 V2 contract 分支忽略 attack_candidate 前的 generic exploit approval 和 Verification 静态一次性 approval；approval 只绑定 exact Candidate plan。legacy static fields 暂时保留给旧 operation，直到单独 contract cleanup migration。`record_finding` 不在三个 V2 specialist tool surface，legacy/non-harness surface 继续由 Task 8 authority guard 兼容。

### 步骤 4：package integration gates

```bash
cd backend && cargo nextest run -p golish-db -E 'test(attack_candidate) | test(candidate_attempt) | test(attack_wave) | test(fact_delta) | test(finding_lineage)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-kit -E 'test(attack_execution) | test(review_barrier) | test(verification_gate) | test(chain_wave)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-runtime candidate_verification --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-app -E 'test(attack_review) | test(candidate_attempt)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-sub-agents -E 'test(attack_analyst) | test(candidate_verifier)' --no-tests=fail --status-level fail
pnpm exec vitest run frontend/components/Engagement/AttackCandidateReview.test.tsx frontend/components/Engagement/CandidateAttemptRows.test.tsx frontend/services/ai-events/harness-handlers.test.ts
just check-fe
cd backend && cargo clippy -p golish-db -p golish-agent-kit -p golish-agent-runtime -p golish-agent-app -p golish-sub-agents -p golish-pentest-domain -p golish-pentest -p golish-pentest-app -p golish-recon-app -p golish-scan-runner -p golish --all-targets -- -D warnings
cargo fmt --all -- --check
python3 scripts/check_repo_ownership.py
```

### 步骤 5：更新模块卡与提交

更新：

- `docs/modules/backend/golish-db/repo.md`
- `docs/modules/backend/golish-agent-kit/harness.md`
- `docs/modules/backend/golish-agent-kit/task_orchestrator.md`
- `docs/modules/backend/golish-agent-runtime/agentic_loop.md`
- `docs/modules/backend/golish-agent-app/ai.md`
- `docs/modules/backend/golish-sub-agents/defaults.md`
- `docs/modules/backend/golish-sub-agents/executor.md`
- `docs/modules/backend/golish-pentest/evidence_ledger.md`
- `docs/modules/backend/golish-pentest-app/findings.md`
- `docs/modules/backend/golish-scan-runner/nuclei.md`
- `docs/modules/frontend/components.md`
- `docs/modules/frontend/lib.md`
- `docs/modules/INDEX.md`

```bash
git add -- frontend/components/Engagement/AttackCandidateReview.tsx frontend/components/Engagement/AttackCandidateReview.test.tsx frontend/components/Engagement/CandidateAttemptRows.tsx frontend/components/Engagement/CandidateAttemptRows.test.tsx frontend/components/ToolCallDetailView/ToolCallDetailView.tsx frontend/services/ai-events/harness-handlers.ts frontend/services/ai-events/harness-handlers.test.ts frontend/store/stage-run.test.ts frontend/store/slices/session-core.ts frontend/store/types/session.ts frontend/lib/api/attack.ts resources/harness/stages/vuln_triage/spec.json resources/harness/stages/vuln_triage/methodology.md resources/harness/stages/attack_candidate/spec.json resources/harness/stages/attack_candidate/methodology.md resources/harness/stages/verification/spec.json resources/harness/stages/verification/methodology.md resources/harness/graph/phases.json backend/crates/golish-sub-agents/src/executor_types.rs backend/crates/golish-sub-agents/src/defaults/builder/mod.rs backend/crates/golish-sub-agents/src/defaults/builder/registry.rs backend/crates/golish-sub-agents/src/defaults/prompts/orchestration.rs backend/crates/golish-sub-agents/src/defaults/tests.rs docs/modules/INDEX.md docs/modules/backend/golish-db/repo.md docs/modules/backend/golish-agent-kit/harness.md docs/modules/backend/golish-agent-kit/task_orchestrator.md docs/modules/backend/golish-agent-runtime/agentic_loop.md docs/modules/backend/golish-agent-app/ai.md docs/modules/backend/golish-sub-agents/defaults.md docs/modules/backend/golish-sub-agents/executor.md docs/modules/backend/golish-pentest/evidence_ledger.md docs/modules/backend/golish-pentest-app/findings.md docs/modules/backend/golish-scan-runner/nuclei.md docs/modules/frontend/components.md docs/modules/frontend/lib.md
git diff --cached --check
git commit -m "feat(ui): expose candidate review attempts and residuals"
```

---

## Task 12：最后才做 per-operation cutover 与最终门禁

**文件：** 新建 `backend/crates/golish-db/migrations/20260712000005_attack_execution_v2_cutover.sql`；修改 `backend/crates/golish-db/tests/attack_execution_v2_migrations.rs`、`backend/crates/golish-agent-kit/src/harness/attack_execution/tests.rs`、`feature_list.json`、`agent-progress.md` 和 Task 11 列出的模块卡。不得在 Task 0-11 提前把 deployment default 设为 V2。

### 步骤 1：写 rollout RED

```rust
#[tokio::test]
async fn existing_legacy_operation_never_changes_contract_when_default_advances() {}

#[tokio::test]
async fn new_v2_operation_uses_only_v2_scope_worker_candidate_and_gate_truth() {}

#[tokio::test]
async fn v2_missing_snapshot_or_repo_data_blocks_without_legacy_field_mix() {}

#[tokio::test]
async fn dual_write_shadow_mismatch_prevents_default_promotion() {}
```

### 步骤 2：跑三阶段 shadow/cutover fixture

只用 fixture DB/fake verifier：

1. `dual_write_read_legacy`：比较 legacy/V2 candidate decisions、review counts，不执行 verifier。
2. `dual_write_read_v2_fallback`：V2 whole-record read；故意删一条 V2 child，确认整条 fallback/trace mismatch，不拼字段。
3. `v2_only`：新 operation 全链路；缺 snapshot/work item/evidence/terminal truth 必须 BLOCK。

只有前两阶段 comparison fixture 全等，才创建 `20260712000005_attack_execution_v2_cutover.sql`。migration 使用三个受 `rank + 1` trigger 保护的 guarded UPDATE，按 `legacy -> dual_write_read_legacy -> dual_write_read_v2_fallback -> v2_only` 推进 singleton；每一步验证 row count=1。它不 UPDATE 任何 `operation_state` 行，旧 operation contract 不变。

### 步骤 3：全量门禁

```bash
./init.sh
just precommit
git diff --check
jq empty feature_list.json
python3 scripts/check_repo_ownership.py
```

把命令、exit code 和关键输出写入 `agent-progress.md`；逐项填 `feature_list.json.verification/evidence`。没有新鲜 `just precommit` 全绿证据时保持 `in_progress`，不得改 `passing`。

### 步骤 4：clean-state checklist 与最终提交

```bash
git status --short
git diff --stat
git diff --check
```

对照 `clean-state-checklist.md`；列出任何未提交文件。确认 scope 仅含 P1/P2 计划内改动后：

```bash
git add -- backend/crates/golish-db/migrations/20260712000005_attack_execution_v2_cutover.sql backend/crates/golish-db/tests/attack_execution_v2_migrations.rs backend/crates/golish-agent-kit/src/harness/attack_execution/tests.rs feature_list.json agent-progress.md docs/modules/INDEX.md docs/modules/backend/golish-db/repo.md docs/modules/backend/golish-agent-kit/harness.md docs/modules/backend/golish-agent-kit/task_orchestrator.md docs/modules/backend/golish-agent-runtime/agentic_loop.md docs/modules/backend/golish-agent-app/ai.md docs/modules/backend/golish-sub-agents/defaults.md docs/modules/backend/golish-sub-agents/executor.md docs/modules/backend/golish-pentest/evidence_ledger.md docs/modules/backend/golish-pentest-app/findings.md docs/modules/backend/golish-scan-runner/nuclei.md docs/modules/frontend/components.md docs/modules/frontend/lib.md
git diff --cached --check
git commit -m "feat(attack): cut new operations over to candidate pipeline v2"
```

不 push；除非用户另行明确授权。

---

## 2. 验收矩阵

| 场景 | 必须结果 |
|---|---|
| sibling org 同 target/hypothesis hash | 两条 V2 Candidate，不冲突、不越权 |
| 删除 live organization/target | audit rows 留存，`target_live_id=NULL`，冻结 identity 可读 |
| hostile sibling Approval/Attempt/Evidence/Lineage | DB constraint/trigger 拒绝 |
| scanner hit | observation seed + evidence；无 Candidate/Finding |
| scanner checked empty | evidence-backed work item 可作 no-candidate；不等于未检查 |
| Attack Candidate Gate BLOCK | Candidate 权威表零新增 |
| empty Candidate deliverable + pending work item | BLOCK |
| complete no-candidate manifest | review close 后 Verification 可 deterministic skip/PASS |
| approval plan hash 漂移/过期/撤销 | action 前拒绝；review reopen 或 Attempt blocked，按生命周期处理 |
| 两进程并发 | 一个 WorkerRun+lane claim；无第二套 Attempt lease |
| verifier 请求 background | `ATTACK_VERIFIER_FOREGROUND_REQUIRED` |
| crash after action started | 同 Attempt 恢复；unknown side effect 不盲重放 |
| verified 缺 proof/finding draft | BLOCK，无 Finding |
| refuted 缺 refutation | BLOCK |
| blocked 缺 reason/evidence | BLOCK |
| terminalizer 重放 | 同 Finding/lineage，不重复 |
| 漏 trace/app restart | review/attempt UI 从 DB 恢复并可 resume |
| FactDelta 无 evidence/foreign ref | 拒绝，不开 wave |
| fuel cap | 不开 wave，持久化 reportable residual risk |
| 旧 operation + 新 default | contract 不变，继续 legacy |
| V2Only 缺 DB truth | fail closed，不混 legacy 字段 |

## 3. 明确不在本计划做的事

- 不把 process-only `BackgroundJobManager` 改成 durable job platform；MVP verifier 采用 foreground contract。
- 不清理 legacy Candidate/schema/state_blob 字段；contract migration 需在 V2 稳定后另立设计与计划。
- 不执行真实 exploit、扫描或外部请求。
- 不让 RAG/KG/memory 成为 scope、approval、Gate、Finding 或 FactDelta 的权威来源。
- 不把 post-exploit/cleanup/reporting domain 强塞进 Candidate 表；本计划只持久化 attack policy residual，供后续专门阶段读取。
