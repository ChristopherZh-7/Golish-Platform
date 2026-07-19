# Organization Delete Manifest Convergence Design

**Date:** 2026-07-18
**Status:** Approved for implementation by the user
**Scope:** Durable organization hard-delete transaction; no schema or migration change

## 1. Problem

The organization deletion job freezes the organization subtree and Targets, performs
external artifact cleanup, then calls `organization_deletion_jobs::hard_delete`.
That transaction validates deletion preconditions and currently executes
`DELETE FROM organizations` for the frozen root.

The Enumeration surface-manifest migration added:

- `fingerprint_origin_observations.organization_id ON DELETE RESTRICT`;
- `enumeration_endpoint_observations.organization_id ON DELETE RESTRICT`;
- a `target_id ON DELETE CASCADE` on both observation families.

Because the hard-delete removes the root organization before explicitly removing its
Targets, PostgreSQL checks the organization RESTRICT edge and aborts before the Target
cascade can remove the observation rows.

## 2. Runtime Evidence

Deletion job `44189bb7-cee5-4bcb-8305-a3c53fc2caf4` froze root organization
`43aeadcd-1f73-4114-be92-4dd253ef41ba` and 18 Targets. Artifact cleanup succeeded and
removed 17 paths. Database hard-delete then failed eight times on
`fingerprint_origin_observations_organization_id_fkey`; the organization and all 18
Targets remained live. The organization owns 256 fingerprint-origin observations.

## 3. Goals

- A durable organization deletion job converges when either manifest observation table
  contains rows for any frozen subtree organization.
- Hard-delete uses the job's frozen organization membership, not caller-supplied live
  scope.
- Existing target-owned cascades remove current identity/content observations before
  organization deletion checks the RESTRICT edges.
- The organization delete and job transition to `hard_delete_committed` remain one
  transaction.
- Historical deletion job snapshots, audit history and retained security history remain
  intact according to their existing FK policies.

## 4. Non-Goals

- No FK alteration and no new migration.
- No manual deletion of live test data.
- No weakening of cleanup-obligation preconditions.
- No change to the single-Target API or its ownership checks.
- No direct deletion of evidence ledger/history rows outside existing target FK policy.

## 5. Design

After locking the job, loading its frozen `organization_ids`, and passing
`assert_deletion_preconditions`, the same transaction deletes the still-live Target
identities captured by this job's immutable target snapshot:

```sql
DELETE FROM targets AS target
USING organization_deletion_job_targets AS frozen
WHERE frozen.job_id = $1
  AND frozen.live_target_id = target.id
```

It then executes the existing root organization delete. Deleting Targets first causes
both manifest observation families to follow their existing `target_id ON DELETE
CASCADE` edges. Other target references continue to use the already-reviewed CASCADE or
SET NULL policies.

The exact target membership comes from `organization_deletion_job_targets`; its
`live_target_id` is allowed to become `NULL` only as a consequence of Target deletion,
while the immutable target-at-time fields remain retained. Frozen organization
membership still comes from `organization_deletion_job_units`, so a reparented or newly
attached organization cannot be silently included. Existing request-time drift guards
and hard-delete preconditions remain authoritative.

No external artifact call occurs inside this transaction.

## 6. Atomicity and Retry

Target deletion, root organization deletion, job state transition and state-history
append commit together. If any FK or trigger rejects the operation, all database deletes
roll back and the durable job remains `artifact_cleanup_succeeded` for bounded backoff.

The already-existing live job therefore needs no manual repair: after a fixed binary is
running, its normal retry can converge from the retained frozen snapshot.

## 7. Failure Semantics

- A non-ready or stale job still fails through the existing state/row lock guards.
- A cleanup-obligation precondition failure occurs before any Target is deleted.
- Manifest rows that do not belong to a frozen Target are not deleted through this path;
  their organization RESTRICT edge correctly continues to block inconsistent data.
- A Target inserted after the frozen deletion request is detected by existing subtree/
  target drift protections rather than being silently deleted.

## 8. Verification

- RED/GREEN integration: legal fingerprint-origin and endpoint/parameter manifest rows
  exist for a frozen Target; artifact cleanup succeeds; hard-delete removes the Target,
  manifest rows and organization, and commits the job.
- Regression: existing precondition and idempotent hard-delete tests remain green.
- Focused `golish-db` nextest for organization deletion and manifest tests.
- Scoped `cargo clippy -p golish-db --all-targets -- -D warnings`, rustfmt check and diff
  checks. No full-workspace gate unless separately authorized.
