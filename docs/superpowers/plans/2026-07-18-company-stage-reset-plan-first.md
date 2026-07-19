# Company Stage Reset Plan-First Convergence Implementation Plan

> **For AI agents:** Required subskills: use `superpowers:test-driven-development` for every behavior change, then `superpowers:verification-before-completion` before changing feature status.

**Goal:** Make a V2-only Company-stage developer reset produce a TeamPlan-first replacement and terminalize any superseded in-flight tool with explicit unknown-outcome history.

**Architecture:** Keep `supersede_stage_checkpoint` as the single short transaction. It closes exact active tool rows while their Worker fence is still intact, supersedes the old runtime, creates replacement Units only, and leaves TeamPlan, WorkItem, message-chain, and Worker creation to the existing canonical Team seed/claim path.

**Tech stack:** Rust 2021, sqlx/PostgreSQL, embedded-Postgres integration tests, cargo-nextest.

---

### Task 1: Add the TeamPlan-first and active-tool reset regression

**Files:**
- Modify: `backend/crates/golish-db/tests/runtime_memory_worker_transactions.rs`

**Step 1: Write the failing integration test**

Add `v2_only_company_stage_reset_is_plan_first_and_closes_active_tool` immediately after `v2_only_developer_reset_ignores_caller_legacy_checkpoint_namespaces`.

Use `create_claimed_compound_runtime_with_contract(...V2Only)`, then create and bind a running tool:

```rust
let tool_call_id = tool_calls::record_tracked_start(
    db.pool(),
    "reset-active-vuln-tool",
    runtime.roots.session_id,
    Some(runtime.roots.operation_id),
    None,
    "vuln_nuclei_general",
    &serde_json::json!({"targets": ["https://delete.example.test"]}),
    Some(&tool_calls::RuntimeToolIdentity {
        operation_id: runtime.roots.operation_id,
        stage_execution_id: runtime.roots.stage_execution_id,
        stage_run_unit_id: Some(runtime.unit_id),
        worker_run_id: Some(runtime.worker_id),
        organization_id: Some(runtime.roots.organization_id),
        attempt_epoch: Some(runtime.worker.attempt_epoch),
        lease_token: runtime.worker.lease_token,
    }),
).await.expect("record reset fixture tool");
runtime_memory_tx::begin_worker_tool(db.pool(), &fence_for_claimed(&runtime), tool_call_id)
    .await
    .expect("bind reset fixture tool");
```

Reset to `vuln_triage` with specialist `vuln_scanner` and a fresh execution. Assert the old Worker is superseded with both active-tool fields null, the tool is failed with `kind=runtime_stage_checkpoint_superseded` and `outcome=unknown_not_replayed`, and the replacement execution contains zero Workers before seed.

Clone `stage_team_controller_seed`, retarget its base identity to the replacement Vuln execution, set generations to 1, the allowed roles to `company_stage_controller` plus `vuln_scanner`, and seed/claim it. Assert one plan exists and claim creates the sole `company_stage_controller` Worker bound to `leader:primary`.

**Step 2: Run the focused test and record RED**

Run:

```bash
just space-guard
cd backend && cargo nextest run -p golish-db -E 'test(v2_only_company_stage_reset_is_plan_first_and_closes_active_tool)' --status-level fail
```

Expected: FAIL before production edits, either because the superseded tool remains running/linked or because Team seed returns `STAGE_TEAM_PLAN_MUST_PRECEDE_WORKERS`.

### Task 2: Make reset terminalize exact active tools and create Units only

**Files:**
- Modify: `backend/crates/golish-db/src/repo/runtime_memory_tx.rs`
- Modify: `backend/crates/golish-db/src/repo/tasks.rs`

**Step 1: Terminalize the exact active tool before Worker supersede**

In `supersede_stage_checkpoint`, lock the affected non-superseded Workers before updating them. For each Worker with `active_tool_call_id`, CAS the exact owner tuple:

```sql
UPDATE tool_calls
   SET status='failed', result=$2, updated_at=NOW()
 WHERE id=$1
   AND worker_run_id=$3
   AND operation_id=$4
   AND stage_execution_id=$5
   AND stage_run_unit_id=$6
   AND organization_id=$7
   AND attempt_epoch=$8
   AND lease_token=$9
   AND status IN ('received','running')
```

Require one updated row per non-null active pointer. Serialize this bounded result:

```rust
serde_json::json!({
    "kind": "runtime_stage_checkpoint_superseded",
    "outcome": "unknown_not_replayed",
    "reason": "developer_reset",
    "schema_version": 1,
})
```

**Step 2: Clear the Worker active-tool fields during supersede**

Extend the existing Worker update with:

```sql
active_tool_call_id=NULL, active_tool_started_at=NULL,
```

Keep lease clearing, terminal timestamp, history retention, legacy mirror sampling, and transaction boundaries unchanged.

**Step 3: Remove replacement Worker precreation**

Keep `stage_run_units::insert_with_executor` for each selected frozen organization, but remove the `stage_worker_runs::insert_with_executor` branch. Update `SupersedeStageCheckpointRow.replacement_specialist` docs to promise queued Units only.

**Step 4: Update the internal reset expectation**

In the internal `cli_descendants_share_one_operation_and_snapshot` test, update the second reset wording and expected `workers_superseded` count so it no longer relies on reset-precreated Workers.

Extend `V2_RELATIONAL_RECOVERABLE_SQL` with a narrow plan-first reset branch: a
specialist Unit may have neither TeamPlan nor Worker only when the server-authored
`runtime_v2_dev_reset` marker names the exact active replacement execution and Unit
stage. Preserve every existing scope, cardinality, and stray-row fence.

**Step 5: Run focused GREEN and adjacent reset coverage**

Run:

```bash
cd backend && cargo nextest run -p golish-db -E 'test(v2_only_company_stage_reset_is_plan_first_and_closes_active_tool) | test(v2_only_developer_reset_ignores_caller_legacy_checkpoint_namespaces) | test(cli_descendants_share_one_operation_and_snapshot)' --status-level fail
```

Expected: all selected tests pass.

### Task 3: Verify reset code quality

**Files:**
- Verify: `backend/crates/golish-db/src/repo/runtime_memory_tx.rs`
- Verify: `backend/crates/golish-db/src/repo/tasks.rs`
- Verify: `backend/crates/golish-db/tests/runtime_memory_worker_transactions.rs`

Run the affected crate checks after the deletion task is also green so compilation is shared:

```bash
cd backend && cargo clippy -p golish-db --all-targets -- -D warnings
cargo fmt --manifest-path backend/Cargo.toml --all -- --check
```

Do not stage or commit in the current shared dirty worktree: both reset files contain unrelated existing user changes. Record exact changed files and verification evidence in `agent-progress.md` instead.
