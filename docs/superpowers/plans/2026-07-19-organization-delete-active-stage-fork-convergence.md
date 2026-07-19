# Organization Delete / Active Stage Fork Convergence Implementation Plan

> **For AI agents:** Required subskills: use `superpowers:test-driven-development` for each behavior, `superpowers:executing-plans` to execute the tasks, and `superpowers:verification-before-completion` before changing feature status.

**Goal:** Prevent organization deletion and active stage forks from deadlocking each other, expose an actionable blocker, and safely converge the already-accepted deletion job.
**Architecture:** Both protocol entry transactions lock live Organizations in the same deterministic order and then reject the other protocol's committed active state; their existing Target locks remain the immutable snapshot/freeze authority. A forward PostgreSQL migration mirrors the stage-fork-side check as a DB invariant, while typed errors travel through Cleanup and Tauri to the Target dialog. Existing outcome-unknown recovery remains the only way to close an unresolved external tool.
**Tech stack:** Rust 2021, sqlx/PostgreSQL, Tauri 2, React 19/TypeScript, Vitest, cargo-nextest.

**Completion:** All tasks completed on 2026-07-19; focused tests/checks passed and live job `44d3b647-a462-47a7-8083-30b6f84ad7ec` reached `hard_delete_committed`.

---

### Task 1: Record the active-fork delete regression

**Files:**
- Modify: `backend/crates/golish-db/tests/operation_stage_forks.rs`

**Steps:**

1. In the existing real fork integration, after the active fork and Target snapshot are committed but before its Task becomes terminal, request deletion of the root organization.
2. Assert the call fails, identifies the exact target operation, and leaves zero active deletion jobs.
3. Run RED:

```bash
just space-guard
cd backend && cargo nextest run -p golish-db --test operation_stage_forks -E 'test(shared_db_candidate_fork_materializes_scoping_prefix_targets_and_wave_entry)' --status-level fail
```

Expected: FAIL because the old request accepts the deletion job.

### Task 2: Add typed delete preflight and the reverse materializer fence

**Files:**
- Modify: `backend/crates/golish-db/src/error.rs`
- Modify: `backend/crates/golish-db/src/repo/organization_deletion_jobs.rs`
- Modify: `backend/crates/golish-db/src/repo/operation_stage_forks.rs`
- Modify: `backend/crates/golish-db/tests/operation_stage_forks.rs`

**Steps:**

1. Add this DB error variant:

```rust
OrganizationDeletionActiveStageFork {
    operation_id: uuid::Uuid,
    stage: String,
    status: String,
}
```

Its display text contains `organization_delete_active_stage_fork` and all fields.

2. After deletion Organization/Target `FOR UPDATE` locks, join fork target-scope organization units to tasks, filter `created|running|waiting`, order deterministically, and return the typed error before snapshots or invalidations are written.
3. After fork target-scope Organization `FOR SHARE` locks, query non-terminal deletion job units for the selected organizations and return:

```rust
OperationStageForkError::Conflict {
    code: "stage_fork_target_organization_deleting",
}
```

4. After terminalizing the first fork in the integration, create the deletion job and try creating a second fork from the same valid source authority. Assert the creation transaction rolls back with `stage_fork_target_organization_deleting`.
5. Re-run the Task 1 command. Expected: PASS.

### Task 3: Add the database-level stage-fork admission guard

**Files:**
- Create: `backend/crates/golish-db/migrations/20260719000001_organization_delete_stage_fork_convergence.sql`

**Steps:**

1. Use `CREATE OR REPLACE FUNCTION validate_operation_stage_fork_target()` and preserve every existing live Target identity comparison.
2. After locking the Target row, add:

```sql
IF EXISTS (
    SELECT 1
      FROM organization_deletion_job_units AS unit
      JOIN organization_deletion_jobs AS job ON job.id=unit.job_id
     WHERE unit.organization_id_at_time=NEW.organization_id
       AND job.state<>'hard_delete_committed'
) THEN
    RAISE EXCEPTION 'stage fork Target organization deletion in progress'
        USING ERRCODE='55000';
END IF;
```

3. Re-run the Task 1 command on fresh embedded PostgreSQL. Expected: migration applies and the test passes.

### Task 4: Carry the blocker through Cleanup and Tauri

**Files:**
- Modify: `backend/crates/golish-cleanup-domain/src/obligation.rs`
- Modify: `backend/crates/golish-cleanup-app/src/ports.rs`
- Modify: `backend/crates/golish-app-core/src/error.rs`
- Modify: `backend/crates/golish-recon-app/src/organizations/mod.rs`

**Steps:**

1. Add `OrganizationDeletionActiveStageFork { operation_id, stage, status }` to `CleanupError` and `GolishError`.
2. Map the DB variant in `repository_error` and the Cleanup variant in `organization_delete` without string parsing.
3. Make `GolishError::code()` return `ORGANIZATION_DELETE_ACTIVE_STAGE_FORK`; extend the serialization tests with the exact operation id.
4. Run:

```bash
cd backend && cargo nextest run -p golish-app-core -E 'test(code_is_stable_per_variant) | test(serializes_with_code_and_message)' --status-level fail
```

Expected: PASS.

### Task 5: Render an actionable Target dialog error

**Files:**
- Modify: `frontend/lib/api/error-codes.ts`
- Modify: `frontend/lib/api/error-codes.test.ts`
- Modify: `frontend/components/TargetPanel/TargetGroupedView.tsx`
- Modify: `frontend/components/TargetPanel/TargetGroupedView.delete.test.tsx`

**Steps:**

1. Add a failing UI test that rejects `deleteOrganization` with an `ApiError` code `ORGANIZATION_DELETE_ACTIVE_STAGE_FORK` and expects:

```text
This organization has an active stage task. Finish or stop that task before deleting.
```

It must not poll organizations or reload Targets.

2. Run RED:

```bash
pnpm exec vitest run frontend/components/TargetPanel/TargetGroupedView.delete.test.tsx frontend/lib/api/error-codes.test.ts
```

3. Add the stable code/translation. In the component, translate `ApiError`; retain `String(error)` only for legacy errors.
4. Re-run the command. Expected: PASS.

### Task 6: Verify affected code

**Files:** Verify every file from Tasks 1-5.

**Steps:**

1. Run scoped Rust checks:

```bash
just space-guard
cd backend && cargo clippy -p golish-db -p golish-cleanup-domain -p golish-cleanup-app -p golish-app-core -p golish-recon-app --lib --tests -- -D warnings
cargo fmt --manifest-path backend/Cargo.toml --all -- --check
```

2. Run scoped frontend checks:

```bash
pnpm exec biome check frontend/lib/api/error-codes.ts frontend/lib/api/error-codes.test.ts frontend/components/TargetPanel/TargetGroupedView.tsx frontend/components/TargetPanel/TargetGroupedView.delete.test.tsx
pnpm exec tsc --noEmit --pretty false
```

3. Run metadata checks:

```bash
jq empty feature_list.json
test "$(jq '[.features[] | select(.status == \"in_progress\")] | length' feature_list.json)" -eq 1
git diff --check
```

### Task 7: Recover the live deletion job without replay

**Files:** No repository edit; operate only on the exact live identities recorded in the design.

**Steps:**

1. Use the existing `resolve_stage_team_recovery` repository/API for Worker `ce4fd12c-99ab-4880-8d1b-af707b6079d7`, with its current read-model CAS. Resolution is `mark_blocked_outcome_unknown`; never replay the tool.
2. Read all Workers for operation `a8c3469f-e505-45bd-b2fe-d169de267504`. Require zero live leases and only the expected abandoned parent `stage_run` tool, mark that parent tool failed, then use the existing Task failed transition with a result identifying organization deletion recovery.
3. Let Cleanup retry job `44d3b647-a462-47a7-8083-30b6f84ad7ec`. Verify it reaches `hard_delete_committed`, organization `38c070ba-55a5-43ee-b4ba-99cfa91cc1fb` disappears, and its 18 live Targets are gone.
4. If a different deterministic FK blocker appears, stop and diagnose it instead of deleting around it.

### Task 8: Update system-of-record documentation

**Files:**
- Modify relevant cards under `docs/modules/backend/` and `docs/modules/frontend/`
- Modify: `docs/modules/INDEX.md`
- Modify: `agent-progress.md`
- Modify: `feature_list.json`

Record RED/GREEN command ids, exit codes, live recovery evidence, risks, and every uncommitted file. Mark the feature `passing` only after focused tests and live deletion readback both exist.
