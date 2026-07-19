# Organization Delete Manifest Convergence Implementation Plan

> **For AI agents:** Required subskills: use `superpowers:test-driven-development` for every behavior change, then `superpowers:verification-before-completion` before changing feature status.

**Goal:** Let the durable organization hard-delete job converge when frozen Targets own Enumeration fingerprint and endpoint manifest observations.

**Architecture:** Preserve the existing request, drift guard, external-cleanup, and hard-delete states. Inside the DB-only hard-delete transaction, delete exactly the job snapshot's still-live Targets before deleting its root organization. Existing target cascades remove current manifest observations; any failure rolls the full transaction back for retry.

**Tech stack:** Rust 2021, sqlx/PostgreSQL, embedded-Postgres integration tests, cargo-nextest.

---

### Task 1: Extend the real two-phase deletion regression

**Files:**
- Modify: `backend/crates/golish-db/tests/cleanup_obligation_kernel.rs`

**Step 1: Give the existing deletion test a sealed operation scope**

In `deletion_request_freezes_targets_before_external_cleanup_and_hard_delete`, replace the manual root project/organization setup with:

```rust
let scope = frozen_scope(&db, &project_path, "two-phase-delete").await;
let organization_id = scope.organization_id;
```

Keep the child organization, unrelated external organization, Target, real request/claim/complete lifecycle, drift assertions, and hard-delete call.

**Step 2: Publish legal manifest rows for the frozen Target**

After the Target is inserted and before the deletion request, insert a matching `web_origins` row and `web_origin_observations` row, a target fingerprint and API endpoint, then publish:

```text
fingerprint_origin_observations(target, fingerprint, web_origin, organization, project)
enumeration_endpoint_observations(operation, target, endpoint, web_origin, organization, project)
enumeration_endpoint_parameters(endpoint observation, query parameter)
```

Use `scope.operation_id`, which has a sealed scope containing `organization_id`; keep origin, endpoint URL, Target organization, and project path exact so all production triggers pass.

**Step 3: Add terminal assertions**

After hard-delete, assert root and child organization, Target, and both manifest observation families are gone; assert the external organization remains; assert the retained job-target snapshot row remains with `live_target_id IS NULL`; assert job state is `hard_delete_committed`.

**Step 4: Run the focused test and record RED**

Run:

```bash
just space-guard
cd backend && cargo nextest run -p golish-db -E 'test(deletion_request_freezes_targets_before_external_cleanup_and_hard_delete)' --status-level fail
```

Expected: FAIL before production edits at `hard_delete` with `fingerprint_origin_observations_organization_id_fkey` (or the equivalent endpoint organization RESTRICT edge). The job transaction rolls back.

### Task 2: Delete frozen live Targets before the root organization

**Files:**
- Modify: `backend/crates/golish-db/src/repo/organization_deletion_jobs.rs`

**Step 1: Add the exact snapshot-owned Target delete**

Immediately after `assert_deletion_preconditions` succeeds, execute in the same transaction:

```rust
sqlx::query(
    r#"DELETE FROM targets AS target
       USING organization_deletion_job_targets AS frozen
       WHERE frozen.job_id=$1
         AND frozen.live_target_id=target.id"#,
)
.bind(job_id)
.execute(&mut *tx)
.await?;
```

Then retain the existing root organization delete, job transition, state history append, and commit. Do not edit the manifest migration or weaken any FK/trigger.

**Step 2: Run focused GREEN and deletion regressions**

Run:

```bash
cd backend && cargo nextest run -p golish-db -E 'test(deletion_request_freezes_targets_before_external_cleanup_and_hard_delete) | test(artifact_cleanup_success_remains_recoverable_until_hard_delete_commits) | test(organization_deletion)' --status-level fail
```

Expected: all selected tests pass.

### Task 3: Verify deletion code quality

**Files:**
- Verify: `backend/crates/golish-db/src/repo/organization_deletion_jobs.rs`
- Verify: `backend/crates/golish-db/tests/cleanup_obligation_kernel.rs`

Share the final `golish-db` Clippy and rustfmt checks with the reset plan. Do not run a full workspace gate without separate user authorization. Do not manually modify the existing live deletion job; the normal retry is expected to converge after the fixed binary starts.

Do not stage or commit in the current shared dirty worktree because the repository contains unrelated existing user changes. Record exact files and verification evidence in `agent-progress.md`.
