# Runtime Memory retained shadow attestation and guarded rollout

**Date:** 2026-07-13
**Scope:** deployment default for runtime-memory contracts; existing operations keep their frozen contract
**Supersedes:** the direct multi-rank cutover described in the rollout section of `2026-07-12-runtime-memory-foundation-corrected.md`

## Problem

`20260712000002_runtime_memory_v2_cutover.sql` runs before any live dual-write
observation exists. A migration can safely enable `dual_write_legacy_read`, but
it cannot prove whole-record parity and must not advance the singleton directly
to `dual_write_v2_preferred` or `v2_only`.

An application-only promoter is also insufficient. The public adjacent CAS and
a raw adjacent `UPDATE runtime_memory_rollout` would otherwise bypass the same
evidence gate. Runtime worker state is mutable, so a one-off in-memory comparison
or an aggregate supplied by the caller is not rollout authority.

## Contract boundary

The rollout remains a deployment default. `operation_state.runtime_memory_contract`
is frozen at operation creation and is never rewritten.

| rank | frozen contract | write authority | whole-record read authority |
|---:|---|---|---|
| 0 | `legacy_v1` | legacy only | legacy |
| 1 | `dual_write_legacy_read` | V2 plus legacy mirror | complete legacy worker record |
| 2 | `dual_write_v2_preferred` | V2 plus legacy mirror | complete V2 worker record, complete legacy fallback only |
| 3 | `v2_only` | V2 only | complete V2 worker record; missing data blocks |

No selector may merge fields from the two records. Existing operations continue
under their frozen contract after the deployment default advances. A late worker
created by such an older operation is therefore allowed to continue and cannot
change the contract frozen into any newer operation.

## Persisted cohort and sample authority

An additive post-foundation migration introduces two immutable ledgers.

1. `runtime_memory_rollout_admissions` assigns a monotonic sequence to every
   dual-contract WorkerRun. An `AFTER WorkerRun INSERT` owner trigger requests
   admission by worker id only; the admission-table prepare trigger overwrites
   even an explicitly supplied sequence, binds the exact persisted
   operation/execution/unit/organization tuple, and reloads the frozen contract,
   rank, rollout version and timestamp. Existing dual WorkerRuns are backfilled
   deterministically when the migration is applied.
2. `runtime_memory_shadow_samples` retains whole-record observations made after
   a dual write. Each row contains the complete persisted legacy worker record,
   the complete V2 worker projection, both hashes, the contract-selected source
   and record, and the equality result. Update/delete is forbidden. The database
   recomputes hashes and validates ownership rather than trusting caller counts
   or booleans.

Every repository path that writes a dual legacy worker mirror records a sample
in the same transaction. This includes seed/claim, checkpoint, heartbeat, tool
start/finish, terminal/retry transitions, final seal, reaping, and developer
reset paths. A missing sample is not equivalent to an equal sample.

## Database-authoritative promotion

Ranks 1 and 2 may advance by one step only when the database gate proves all of
the following for the exact current-contract cohort:

- at least one admitted WorkerRun exists;
- every admission at or below the transaction's cutoff has at least one retained
  sample and no retained mismatch;
- every retained sample still has canonical hashes, identity and selected-source
  semantics;
- the current V2 WorkerRun, frozen organization name and current legacy
  `worker_records[worker_id]` rehydrate to the same complete canonical record;
- every operation/admission contract and rank still match;
- the runtime/attack compatibility matrix remains valid.

The transition trigger itself runs this relational gate. Only after the row has
completed its adjacent transition does an `AFTER UPDATE` owner trigger request a
receipt; a receipt prepare trigger ignores every supplied field and rebuilds the
old rank/contract/version, cutoff, counts, digest and timestamp from the updated
singleton and retained old-contract cohort. Consequently the
repository CAS and raw SQL have the same authority and neither can skip the
attestation. The trigger writes an immutable promotion receipt containing the
from/to contracts, rollout versions, database-generated cutoff and canonical
aggregate digest. The receipt is evidence, not an input to the decision.

The promoter takes the rollout row lock before freezing `MAX(admission_seq)`.
Worker admission takes a share lock on the rollout row. A concurrent admission
therefore lands either before the cutoff and is checked or after promotion under
an already-frozen older operation contract. Promotion does not take operation
row locks: old operations are not switched by the default change, avoiding the
`operation -> rollout` / `rollout -> operation` deadlock.

## Reconciliation boundary

Application code calls a typed, idempotent reconciler only after the dual-write
transaction commits. A not-ready cohort returns `unchanged_not_ready`; it is not
an error and never rolls back durable runtime state. Operation creation may run
the same best-effort reconciliation before starting the transaction that freezes
the singleton. SQL errors are never swallowed inside an aborted transaction.

The startup abandoned-task transaction returns how many runtime shadow samples
it actually wrote. Only a positive count schedules post-commit reconciliation;
replaying startup after response loss therefore cannot manufacture another
sample or promotion receipt. At final-seal adapter boundaries, runtime always
reconciles before the Candidate attack rollout, and both remain independent
best-effort transactions after the business commit.

V2-only developer reset never consumes the caller's legacy checkpoint payload.
It starts from the locked persisted operation blob, removes `graph_flow`, flat
cursor fields, worker/handoff mirrors and legacy schema metadata, preserves
non-checkpoint server namespaces, and writes a server-owned relational reset
marker. This prevents a V2-only operation from recreating a legacy read path.

## Compatibility matrix

Attack execution can depend on runtime V2 but never the reverse:

- attack `legacy` is valid with every runtime contract;
- attack dual contracts require runtime rank at least 1;
- attack `v2_only` requires runtime `v2_only` exactly;
- deployment promotion must reject any next default that would make the pair
  invalid, and operation creation rechecks the same matrix while both rollout
  rows are share-locked.

## Non-goals

- No existing operation contract is rewritten.
- Shadow rows do not become Gate, evidence-ledger, Candidate, or Finding truth.
- No external request, scan, or exploit is part of rollout reconciliation.
- A historical mismatch cannot be deleted or replaced to manufacture readiness.
