# Attack FactDelta Wave Entry 与全局收敛实现计划

> **面向 AI 代理的工作者：** 使用 `executing-plans`、`test-driven-development` 与 `verification-before-completion`；每个 production slice 必须先观察对应 RED。

**目标：** 用 additive `00012` 和一个 DB-authoritative compound transaction 完成 evidence-backed FactDelta acceptance、typed follow-on Wave entry、all-org barrier、fuel/residual 与 exact replay，为 Candidate V2 Task 10–12 提供真实持久化游标。

**架构：** `attack_fact_delta_decisions` 冻结 proposal 验证；`attack_wave_consolidations` 同时是 immutable FactDelta-set header 与 source Wave 的唯一 cursor decision；members 把 delta 复合绑定到 follow-on Unit/work-item 或 exhausted residual。`attack_wave_units` 使用 handoff XOR consolidation entry。`golish-agent-kit` 只消费 typed repo result，不再用 model candidates/process-local chain state 推进 V2。

**约束：** 用户已于 2026-07-13 明确授权 00012 与必要 `golish-db` 改动；不 push，不运行真实攻击，不提前创建/推进 00002 或 00005。

---

## Task A：00012 schema contract（先 RED）

**文件：**

- 新建 `backend/crates/golish-db/migrations/20260712000012_attack_fact_delta_wave_entry.sql`
- 修改 `backend/crates/golish-db/tests/attack_execution_v2_migrations.rs`

### A1. RED

先写并运行：

```rust
#[tokio::test]
async fn attack_wave_entry_is_exactly_one_of_handoff_or_fact_delta_consolidation() {}

#[tokio::test]
async fn consolidation_member_cannot_cross_operation_scope_org_or_target_wave() {}

#[tokio::test]
async fn fact_delta_acceptance_requires_one_immutable_typed_decision() {}
```

```bash
just space-guard
cd backend && cargo nextest run -p golish-db -E 'test(attack_wave_entry_is_exactly_one_of_handoff_or_fact_delta_consolidation) | test(consolidation_member_cannot_cross_operation_scope_org_or_target_wave) | test(fact_delta_acceptance_requires_one_immutable_typed_decision)' --no-tests=fail --status-level fail
```

预期：00012 tables/columns/constraints 不存在，RED；不能用 `--no-tests=pass`。

### A2. GREEN

实现 frozen 00004 之后的 additive ALTER、immutable decision/consolidation/member tables、composite FK、XOR shape、follow-on entry trigger、deferred member-count trigger 和 update/delete rejection trigger。迁移不得 UPDATE rollout singleton 或 operation rows。

重复上面的 focused tests，再跑：

```bash
cd backend && cargo nextest run -p golish-db --test attack_execution_v2_migrations --no-tests=fail --status-level fail
```

---

## Task B：canonical acceptance 与 compound consolidation（先 RED）

**文件：**

- 修改 `backend/crates/golish-db/src/repo/{attack_fact_deltas.rs,attack_waves.rs,attack_candidate_work_items.rs,mod.rs}`
- 新建 `backend/crates/golish-db/src/repo/attack_wave_consolidations.rs`
- 修改 `backend/crates/golish-db/tests/attack_execution_v2_migrations.rs`

### B1. RED

```rust
#[tokio::test]
async fn only_unconsumed_evidence_backed_delta_opens_next_wave() {}

#[tokio::test]
async fn sibling_or_stale_canonical_ref_delta_is_rejected() {}

#[tokio::test]
async fn all_org_units_must_be_terminal_before_global_cursor_advances() {}

#[tokio::test]
async fn fuel_cap_closes_wave_and_persists_reportable_residual_risk() {}

#[tokio::test]
async fn response_loss_replay_returns_the_same_wave_member_and_work_item_dag() {}
```

每个测试必须断言 exact status/reason/hash/FK 与“无局部写”，而不是只断言 `is_err()`。

### B2. 实现

1. 将 string kind 映射到 closed UUID canonical catalog，version 仅接受 1；用 frozen project path/org 重新解析并比较 content hash。
2. 核对 delta evidence 与 exact source Attempt 的 `candidate_attempt_evidence(role='fact_delta')`。
3. 按固定锁序完成 all-org barrier、proposal decisions、fuel counters、next Wave/Unit、delta-backed manifest/member、consume 或 residual、source terminalization、consolidation parent。
4. deterministic UUID/hash、自然键 replay 与 immutable readback 必须覆盖 response loss。
5. 零 delta org 创建 terminal no-input Unit，不创建空 manifest。

### B3. GREEN

```bash
just space-guard
cd backend && cargo nextest run -p golish-db -E 'test(fact_delta) | test(attack_wave) | test(consolidation) | test(residual)' --no-tests=fail --status-level fail
cd backend && cargo clippy -p golish-db --all-targets -- -D warnings
cargo fmt --all -- --check
```

---

## Task C：kit/bridge 与 durable V2 graph flow（逐层 RED）

**文件：**

- 新建 `backend/crates/golish-agent-kit/src/harness/attack_execution/fact_delta.rs`
- 修改 `backend/crates/golish-agent-kit/src/harness/attack_execution/{mod.rs,types.rs,tests.rs}`
- 修改 `backend/crates/golish-agent-kit/src/{db_traits.rs,harness/operation_flow.rs,harness/phase_flow.rs}`
- 修改 `backend/crates/golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`
- 修改 `backend/crates/golish-agent-app/src/ai/db_bridge/{attack_execution.rs,runtime_memory.rs}`

### C1. RED/GREEN contract

- exact Verification PASS 先 durable-close 本 org verification，再尝试 global consolidation；未满足 barrier 返回 waiting，不走 legacy fallback。
- `opened_next_wave` 把 V2 cursor 回到 `attack_candidate` 并绑定 target Wave；`closed_no_delta|exhausted` 走正常 DAG。
- V2 repo missing/error 一律 BLOCK，不能读取或拼接 deliverable candidates、`spawned_candidates`、legacy `chain_wave_seen`。
- chat resume、CLI/stage_run 与桌面端必须走同一个 trait/bridge result。

```bash
just space-guard
cd backend && cargo nextest run -p golish-agent-kit -E 'test(fact_delta) | test(chain_wave) | test(verification)' --no-tests=fail --status-level fail
cd backend && cargo nextest run -p golish-agent-app -E 'test(attack) | test(candidate) | test(verification)' --no-tests=fail --status-level fail
```

---

## Task D：stage contracts、trace 与安全诊断（先 RED）

**文件：**

- 修改 `resources/harness/stages/{vuln_triage,attack_candidate,verification}/{spec.json,methodology.md}`
- 修改 `resources/harness/graph/phases.json`
- 修改 sub-agent defaults/prompts 与 `HarnessTraceKind`
- 修改 `scripts/run_tree.py`
- 修改 `scripts/tests/test_run_tree_runtime_memory.py`

### D1. RED fixtures

1. happy path：两 Wave、两 org、exact Attempt Worker/lane、consumed delta + consolidation member、reported residual。
2. anomaly/redaction：foreign worker/lane、accepted without decision/evidence、consumed without entry provenance、pending residual；所有敏感字段注入 `SECRET_CANARY`，输出必须不含 canary。

### D2. GREEN

V2 路径忽略 attack_candidate 前 generic exploit approval 与 Verification static approval，只认 exact Candidate plan approval。trace 只发 typed refresh/progress，不充当 Gate truth。

```bash
python3 -m unittest scripts.tests.test_run_tree_runtime_memory
python3 -m py_compile scripts/run_tree.py
```

---

## Task E：rollout shadow 与 cutover（最后执行）

先实现并跑 `dual_write_read_legacy`、`dual_write_read_v2_fallback` 与 `v2_only` fixture。whole-record fallback 不得混字段；missing V2 snapshot/repo data 必须 BLOCK。只有 comparison fixtures 全等才创建缺失的 rollout migration；00005 只能 guarded 单步推进 singleton，不能 UPDATE 既有 operation contract。

若任何 shadow mismatch，保持 feature `in_progress`，记录 blocker，不创建/推进 00005。

---

## Task F：模块卡、全量证据与 commit

同步：

- `docs/modules/backend/golish-db.md`
- `docs/modules/backend/golish-db/repo.md`
- `docs/modules/backend/golish-agent-kit/{harness.md,task_orchestrator.md,db_traits.md}`
- `docs/modules/backend/golish-agent-app/ai.md`
- `docs/modules/backend/golish/stage_run.md`
- 实际改动触及的 frontend/sub-agent/runtime 模块卡
- `docs/modules/INDEX.md`
- `agent-progress.md`、`feature_list.json`

最终门禁：

```bash
./init.sh
just precommit
git diff --check
jq empty feature_list.json
python3 scripts/check_repo_ownership.py
```

只有 verification/evidence、clean-state checklist 与 fresh `just precommit` 全部满足才将 feature 改为 `passing` 并 commit。commit 后不 push。
