# Organization Delete Quiescent Stage Task Convergence

## Problem

Organization deletion currently treats every stage-fork Task in
`created|running|waiting` as active. Closing a ChatPanel tab cancels only the
in-memory AI session and removes the conversation projection; headless CLI
stage-fork Tasks and durable resumable Tasks remain `waiting`. As a result, an
operation with no process, no valid Worker lease and no active tool authority
can retain every frozen Target indefinitely.

The operating-system process list cannot be the deletion authority. A process
may be between restarts, remote from the desktop process, or alive without the
right to commit. The durable database lease, active-tool pointer and Task state
must decide whether an executor can still act.

## User-visible contract

- The organization-delete confirmation states that paused stage Tasks with no
  active executor will be stopped as part of deletion.
- A `created` or `running` stage-fork Task still blocks deletion.
- A `waiting` stage-fork Task still blocks deletion when any Worker has an
  unexpired lease or retains an `active_tool_call_id`, including
  `recovery_required` outcome-unknown work.
- A `waiting` stage-fork Task with no unexpired Worker lease and no active-tool
  pointer is quiescent. The explicit destructive delete confirmation authorizes
  the backend to fail that Task, close its open Turn through the existing Task
  status trigger, fail its stale `received|running` tool rows, and then create
  the two-phase deletion job in the same transaction.
- If any blocker changes while deletion is being admitted, row locks and
  compare-and-set predicates make the request fail closed instead of combining
  a resumed executor with a deletion job.

## Transaction and lock order

1. Resolve and lock the exact Organization subtree and live Targets using the
   existing deletion lock order.
2. Resolve all non-terminal stage forks that reference the subtree.
3. Lock their `operation_state` rows in UUID order, then their Task rows in the
   same order. This matches exact-resume's operation-before-Task order.
4. Lock every corresponding `stage_worker_runs` row and reject if an unexpired
   lease or any active-tool pointer exists.
5. Reject `created|running`; collect only `waiting` Tasks that passed the
   quiescence test.
6. Fail stale `received|running` tool rows for those operations, then fail the
   Tasks with a stable organization-deletion result marker. The existing Task
   trigger closes open `operation_turns` as failed.
7. Insert the deletion job and state history, recording the stopped operation
   identities, before committing the existing invalidation work.

No schema or migration is required. Terminal Task status is already the
authority used by the immutable stage-fork Target trigger, and terminal stage
history remains available for audit.

## Safety invariants

- No active lease is cancelled implicitly.
- No `active_tool_call_id` is overwritten or replayed.
- No Task outside the exact Organization subtree's stage forks is changed.
- Source operations remain unchanged; only the target fork Tasks are stopped.
- Deletion admission and stage-fork creation retain their existing Organization
  lock fence, so only one side of the race can commit.
- The implementation does not inspect or kill OS processes and does not mutate
  the user's current live database during development or verification.

## Verification

- A fresh embedded-Postgres regression first proves that a `waiting`,
  lease-free, pointer-free stage fork is rejected by the old implementation.
- The same regression then proves atomic Task/tool/Turn closure and deletion-job
  admission, while preserving rejection for a non-waiting fork.
- Focused frontend tests prove the destructive confirmation discloses automatic
  paused-task closure and the remaining blocker message describes live or
  recovery-required authority.
- Scoped Rust Clippy/rustfmt, focused Vitest/Biome/TypeScript, JSON and diff
  checks provide the final evidence. Full workspace gates remain opt-in under
  `AGENTS.md` section 0.1.
