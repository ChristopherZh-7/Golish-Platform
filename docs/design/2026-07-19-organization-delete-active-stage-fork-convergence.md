# Organization Delete / Active Stage Fork Convergence Design

**Date:** 2026-07-19
**Status:** Implemented; live deletion recovery verified
**Scope:** organization deletion request, immutable stage-fork Target snapshot, typed IPC error, Target delete dialog, and one already-accepted deletion job

## 1. Problem

Organization deletion and post-Scoping stage forks independently freeze the same live
Target identities:

- a deletion request locks the organization subtree and Targets, commits invalidation
  work, performs external artifact cleanup, and only then hard-deletes the live rows;
- an EAS-or-later stage fork snapshots live Targets and prevents identity/scope changes
  while its Task is `created`, `running`, or `waiting`.

The two protocols did not check each other. A deletion request could therefore be
accepted while an active stage fork still owned the Targets. Artifact cleanup completed,
but the hard-delete transaction's `DELETE FROM targets` hit
`active stage fork Target identity/scope is frozen`. The frontend only waited ten
seconds for the organization row to disappear and reported a generic background
processing message.

Current evidence is deletion job `44d3b647-a462-47a7-8083-30b6f84ad7ec` for root
organization `38c070ba-55a5-43ee-b4ba-99cfa91cc1fb`: artifact cleanup succeeded,
18 Targets remain, and Vuln fork operation
`a8c3469f-e505-45bd-b2fe-d169de267504` is still `waiting`. One exact Worker is
`recovery_required`, so its unknown external-tool outcome must be resolved through the
existing no-replay operator recovery before the Task can be terminalized.

## 2. Root Cause

The Target protection trigger correctly protects an active immutable fork. The bug is
missing coordination at protocol entry:

1. `organization_deletion_jobs::request` locks the live Targets but does not reject an
   already-active stage fork that references them.
2. `operation_stage_forks::materialize_with_connection` locks live Targets but does not
   reject an organization already owned by a committed deletion job.
3. The stage-fork Target validation trigger has the same one-sided blind spot.
4. The Target dialog renders `String(error)` and its timeout text, so a stable backend
   blocker code is not translated into an actionable message.

## 3. Goals

- Reject organization deletion before any invalidation or artifact cleanup when an
  active stage fork freezes any Target in the requested subtree.
- Reject stage-fork materialization when any selected organization already belongs to
  an active organization deletion job.
- Make the two checks race-safe by acquiring the same live Organization row locks in
  deterministic order before checking the other protocol's committed state.
- Preserve the database Target freeze; never bypass or weaken it.
- Return a typed IPC error and show an actionable message in the delete dialog.
- Recover the already-accepted job through existing outcome-unknown recovery, terminal
  Task semantics, and the normal cleanup worker retry.

## 4. Non-Goals

- No automatic replay of an outcome-unknown tool.
- No force-deleting immutable fork snapshots or stage history.
- No blind cancellation of an in-process scanner.
- No change to evidence, Gate, or checked-empty semantics.
- No full-workspace validation without separate authorization under `AGENTS.md` §0.1.

## 5. Bidirectional Admission Fence

### 5.1 Delete request rejects an existing active fork

After the deletion transaction locks the subtree Organizations and current Targets with
`FOR UPDATE`, it queries `operation_org_scope_units -> operation_stage_forks -> tasks`
for the selected organization ids and task status `created|running|waiting`. The first deterministic blocker returns a
typed `DbError::OrganizationDeletionActiveStageFork` carrying operation id, entry stage,
and task status. No deletion job, invalidation event, or artifact cleanup is committed.

The lock ordering makes the check race-safe: a fork that already holds `FOR SHARE` on an
Organization commits first and becomes visible to the delete transaction; a delete
transaction that obtains `FOR UPDATE` first commits its job before a blocked fork can
continue.

### 5.2 Stage fork rejects an active deletion

Before `materialize_with_connection` reads the live Target snapshot, it locks every
selected Organization with `FOR SHARE` and checks whether any occurs in a non-terminal deletion job. It
returns `stage_fork_target_organization_deleting` before provider/tool dispatch and the
caller-owned operation-create transaction rolls back.

A forward migration also updates `validate_operation_stage_fork_target()` with the same
rule. This keeps raw or future writers fail-closed even if they bypass the Rust
materializer. The trigger uses SQLSTATE `55000` and does not alter existing immutable
fork rows.

## 6. Typed Error Path

The blocker remains typed through all layers:

```text
golish-db DbError
  -> cleanup-domain CleanupError
  -> golish-app-core GolishError
  -> { code, message } Tauri envelope
  -> frontend error-code translation
```

The stable IPC code is `ORGANIZATION_DELETE_ACTIVE_STAGE_FORK`. The delete dialog tells
the operator to finish or stop the active stage task before retrying; backend logs retain
the exact operation id/stage/status for diagnosis.

## 7. Existing Job Recovery

The already-accepted job is not rewritten and its successful artifact cleanup is not
replayed. Recovery order is:

1. Resolve every exact `recovery_required` Worker with the existing
   `mark_blocked_outcome_unknown` CAS; this closes the active tool without replay.
2. Verify no live Worker lease or unresolved active tool remains for the blocking fork.
3. Mark the exact Task failed through the existing Task status contract, which also
   closes its open Operation Turn.
4. Let the Cleanup worker retry the retained `artifact_cleanup_succeeded` job.
5. Verify the organization and all 18 live Targets disappeared and the deletion job
   reached `hard_delete_committed`.

## 8. Verification

- RED/GREEN embedded-Postgres integration: an active fork makes deletion request fail
  before a job exists.
- RED/GREEN embedded-Postgres integration: a committed deletion job makes a new fork
  creation fail with `stage_fork_target_organization_deleting`.
- Error serialization tests for the new stable backend code.
- Focused Target dialog Vitest proves the user sees an actionable message.
- Scoped Clippy/rustfmt/Biome/typecheck for affected files only.
- Live readback proves the retained job converges after no-replay recovery and terminal
  Task transition.
