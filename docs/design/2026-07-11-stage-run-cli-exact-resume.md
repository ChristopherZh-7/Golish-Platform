# Headless stage-run exact resume

## Problem

`golish --stage-run` is a one-shot runner. A deterministic stage gate may
interrupt after its request-local continuation budget is exhausted, leaving a
valid `graph_flow` checkpoint, `stage_run_workers` exact-chain map, and a task in
`waiting`. Re-running the original command is not a continuation: it allocates a
new `stage-run-*` chat key, DB session, task, operation, freshness epoch, and
`technique_outcomes.run_id`.

The interactive Task path already has the required primitive:
`TaskOrchestrator::resume(task_id, user_message, executor)`. The missing piece is
a CLI entry point that reconstructs the original session identity and refuses
ambiguous or cross-scope recovery.

## Command contract

```bash
golish --stage-run-resume <stage-run-chat-key|session-uuid|operation-uuid> \
  -e "继续" \
  /absolute/path/to/original/workspace
```

A process killed before the graph can mark the task `waiting` leaves a stale
`running` row. Recovering that state requires an explicit operator assertion and
exact expected identities:

```bash
golish --stage-run-resume <selector> \
  --allow-orphan-running \
  --repair-missing-graph-flow \
  --repair-reaped-task \
  --expect-session <db-session-uuid> \
  --expect-task <task-uuid> \
  --expect-operation <operation-uuid> \
  --expect-org <organization-uuid> \
  --expect-stage <stage-id> \
  -e "继续" \
  /absolute/path/to/original/workspace
```

`--stage-run-resume` implies the headless stage-run bootstrap. It conflicts with
fresh-run selectors (`--stage-run`, `--profile`, `--from`, `--to`, `--only`,
`--org`, `--target`, `--include-subsidiaries`) and ephemeral DB flags. The
continuation message defaults to `继续` when `-e` is omitted.

Selector resolution is deterministic:

- `stage-run-*` resolves the exact `sessions.chat_session_key` row.
- A UUID resolves an `operation_state.operation_id` first; if no operation has
  that id, it resolves a DB `sessions.id`.
- A session selector is accepted only when exactly one status-eligible task with
  an `operation_state` row belongs to it (`waiting`, or explicitly asserted
  orphan `running`). Full checkpoint and chain validation happens immediately
  after selection; multiple candidates require the operation UUID.

## Fail-closed resume preconditions

Resume is allowed only when all of the following hold:

1. The session has a `stage-run-*` chat key.
2. The selected task belongs to that DB session.
3. `operation_state.operation_id == task.id` and the operation is not
   superseded.
4. The task status is exactly `waiting` by default. `running` is rejected unless
   `--allow-orphan-running` is present and the expected DB session, task,
   operation, organization, and current-stage identities are supplied and
   exactly match. A `failed` row is accepted only by the narrow startup-reaper
   repair below; ordinary failed tasks, plus `created` and `finished`, are
   always rejected.
5. `state_blob.graph_flow.state` fully deserializes as `OperationFlowState` and
   `next_node` equals `operation_state.current_stage`. The sole exception is an
   explicitly authorized first-stage/mid-node repair described below.
6. The current stage parses under the persisted profile and is resumed as a
   one-stage allowlist. This closes the interrupted stage without accidentally
   extending a historical `--only` run into later profile stages.
7. `state_blob.stage_run_workers[current_stage]` contains at least one exact
   worker reference. Every referenced chain must exist with the same DB session,
   specialist agent, and non-null persisted chain. Newer rows with `task_id`
   must match the operation; legacy stage-run chains may have `task_id=NULL`
   because the exact operation binding is already provided by the guarded
   `stage_run_workers` map. A non-null mismatching task id is always rejected;
   the CLI never backfills the legacy row.
8. The passed workspace resolves the transcript directory that already contains
   the selected `stage-run-*` session. This prevents silently writing the resumed
   transcript into a different workspace.
9. Explicit provider/model overrides, if supplied, must equal the values stored
   on the original session. Otherwise the runner inherits the stored values.
10. Before the second/final DB read and before changing task status, the runner
    must acquire a non-blocking PostgreSQL advisory lock derived from the exact
    operation UUID. The dedicated lock connection is detached from the pool and
    held for the whole resumed request, so connection/process loss releases the
    claim automatically. A second resume process fails immediately. No DB
    transaction is held across LLM or network work.

## Missing graph-flow checkpoint repair

The initial `run_stage` checkpoint is a flat `HarnessResumeState`. The graph
checkpointer writes its nested `graph_flow` key only after a stage node returns.
Therefore a process killed while the first stage worker is still running can
have a valid flat `current_stage`, `current_stage_run_id`,
`stage_run_workers`, and producer checkpoints but no `graph_flow`; the existing
`latest_resumable_by_session`/`TaskOrchestrator::resume` path cannot see or load
that operation.

`--repair-missing-graph-flow` is an explicit, narrow repair. It is accepted only
when all expected identities are present, the task/operation/session/org/stage
invariants pass, the whole flat blob deserializes as `HarnessResumeState`, its
`profile` and `current_stage` equal the operation row, `current_stage_run_id` is
a non-nil UUID, `completed_count == 0`, exact worker chains pass ownership
checks, and `graph_flow` is absent. After acquiring the operation advisory lock,
the runner performs one compare-and-set update equivalent to:

```sql
UPDATE operation_state
SET state_blob = jsonb_set(state_blob, '{graph_flow}', $checkpoint, true)
WHERE operation_id = $operation
  AND current_stage = $expected_stage
  AND state_blob = $expected_flat_blob
  AND superseded_by IS NULL
  AND state_blob -> 'graph_flow' IS NULL;
```

`$checkpoint` is `{state: OperationFlowState::default(), next_node:
current_stage}`. `jsonb_set` preserves `stage_run_workers`,
`route_probe_checkpoints`, and every other sibling key. Affected rows must equal
one; otherwise recovery stops. The runner then re-reads and fully validates the
now-loadable checkpoint before calling `resume()`.

Any missing, malformed, ambiguous, unasserted-running, terminal, superseded, or
cross-session state is an error before an AI request begins.

## Startup-reaped flat checkpoint repair

Legacy startup cleanup classified a task as resumable only when
`state_blob.graph_flow` already existed. A process killed inside the first stage
can instead have the fully validated flat checkpoint above; startup therefore
marked that exact orphan `failed` with the fixed result
`Abandoned: the process exited before this task finished.` before the resume CLI
could repair `graph_flow`.

`--repair-reaped-task` is a separate explicit capability. It requires all five
expected identities, the exact fixed startup-reaper result, a non-superseded
operation, exact session/profile/stage/org/state-blob equality, and the
operation advisory lock. The CLI compare-and-sets only that row from `failed`
to `waiting`, clears the synthetic abandoned result, then performs the guarded
graph repair and re-reads every invariant. A provider/tool failure or any other
failed-task result is never resurrected. The shared startup reaper also treats
the same complete, `completed_count == 0` flat checkpoint as recoverable so
future first-stage orphans are paused instead of failed.

## Execution identity

The resume branch reuses:

- the original chat key as the bridge event/evidence session id and
  `technique_outcomes.run_id`;
- the original `sessions.id` as the DB tracker persistence session id;
- the original task id as the harness operation id;
- the original `operation_state.stage_started_at` because same-stage entry does
  not advance the cursor;
- the existing transcript directory and exact worker chain.

It acquires a fresh top-level request lease, creates a
`BridgeAgentExecutor::from_request`, sets the persisted profile/org/stage
allowlist, enables the one-shot `stage_run` fast-resume tool lock, and calls
`TaskOrchestrator::resume`. It must never call `run_stage` or insert a new task or
operation.

The existing bridge/session-generation lease is process-local; it cannot prove
that a process killed with `SIGINT` is gone from another process. Consequently,
`running` recovery is never inferred from timestamps. The explicit orphan flag
and exact expected ids are the operator assertion, while the advisory lock is
the atomic cross-process claim among compliant resume callers. The target is
resolved and validated again after the claim is acquired to close the
check-then-act race.

## Non-goals

- No DB schema or migration change.
- No cross-session chain adoption.
- No implicit recovery of `running` tasks and no recovery of `finished` or
  ordinary `failed` tasks; only the exact startup-reaper marker is repairable
  under its dedicated explicit flag and complete identity/CAS checks.
- No automatic selection among multiple waiting operations.
- No mutation or invocation against a currently live stage-run during
  implementation verification.

## Verification

Unit tests cover CLI parsing and pure candidate validation. Existing stage-run
tests plus scoped `cargo clippy -p golish --all-targets -- -D warnings` verify the
integration without connecting to the app database. A real resume invocation is
deliberately excluded from implementation verification because it mutates the
persisted operation and may race the live process.
