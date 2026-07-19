# Company Stage Reset Plan-First Convergence Design

**Date:** 2026-07-18
**Status:** Approved for implementation by the user
**Scope:** Developer stage reset for V2-only Company Controller stages; no schema or migration change

## 1. Problem

`restart_stage`, `restart_from_stage`, and `restart_from_stage_purge` replace the active
stage execution through `runtime_memory_tx::supersede_stage_checkpoint`. For a stage
whose embedded spec declares a specialist, the reset transaction currently creates a
queued `StageRunUnit` and a queued specialist `StageWorkerRun` immediately.

That bootstrap shape predates the Company Controller scheduler. Current company stages
must be seeded in this order:

1. `StageRunUnit`;
2. immutable `StageTeamPlan`;
3. server-seeded `StageWorkItem` rows, including `leader:primary`;
4. a Worker only when a WorkItem is claimed.

The database enforces that order. A reset-created worker therefore makes the next
`seed_stage_team_runtime` call fail with `STAGE_TEAM_PLAN_MUST_PRECEDE_WORKERS` before
provider dispatch.

The reset also supersedes Workers without closing their still-running `tool_calls` or
clearing `active_tool_call_id`. Those rows cannot execute after reset, but they continue
to look live to recovery and audit readers.

## 2. Runtime Evidence

Operation `951cce90-7304-464c-860c-401f698d9e71` replaced Vuln execution
`6e4bd5d4-7a0f-4ecb-87b0-c9163a75c1dc` with
`1d175c2a-5588-462c-8469-5e64303dfa9e` at 2026-07-18 12:48:33 +08:00.
The replacement contained Unit `8f2eb9ca-be85-4a90-b742-7f9b1df525f2` and legacy
Worker `8bccab1f-db2d-4ff6-8c29-e304bd813121`, but no TeamPlan. Four subsequent
`stage_run` calls returned the plan-before-worker constraint error with
`provider_dispatched=false`.

## 3. Goals

- A reset Company stage can be seeded by `seed_stage_team_runtime` immediately.
- The reset transaction never creates a planless Company-stage Worker.
- The canonical seed remains the single owner of TeamPlan, WorkItem and Worker creation.
- Running/received tool calls owned by superseded Workers become explicit failed,
  outcome-unknown history in the same reset transaction.
- Superseded Workers have no live lease or active-tool pointer.
- Runtime history, evidence and prior outputs remain retained; no old row is deleted.
- `restart_from_stage_purge` keeps its existing second, explicit fact-purge transaction.

## 4. Non-Goals

- No migration or trigger change.
- No replay of an outcome-unknown external tool.
- No conversion of an old specialist Worker into a Company Controller.
- No change to ordinary stage transition, Gate repair, successor Turn, or operator
  recovery semantics.
- No change to legacy/dual reset behavior beyond the V2 relational lifecycle already
  owned by this transaction.

## 5. Design

### 5.1 Replacement shape

For V2-only reset with a sealed organization scope, create replacement Units but do not
create replacement Workers. The Unit keeps the embedded specialist value because the
next canonical Team seed validates stage/spec identity and reuses that Unit.

The resulting pre-seed invariant is:

```text
replacement execution: started
replacement unit(s): queued
replacement team plan: absent
replacement work items: absent
replacement workers: absent
```

The next `stage_run` performs the existing canonical Team seed and may then claim the
`leader:primary` WorkItem, producing a Company Controller Worker with a non-null
`work_item_id` and a new message chain.

The V2 resumable-task selector recognizes this Worker-free shape only when the
server-authored `runtime_v2_dev_reset` marker names the exact active replacement
execution and selected stage. This narrow carve-out keeps malformed planless Units
fail-closed while allowing the next chat turn to reach the canonical Team seed.

### 5.2 Superseded active-tool lifecycle

Before updating affected Workers to `superseded`, lock/update every received/running
`tool_calls` row referenced by those Workers. Each becomes `failed` with a bounded
structured result:

```json
{
  "kind": "runtime_stage_checkpoint_superseded",
  "outcome": "unknown_not_replayed",
  "reason": "developer_reset",
  "schema_version": 1
}
```

The update is limited by exact operation, affected execution/unit membership and
`tool_calls.worker_run_id = stage_worker_runs.id`. It does not rewrite already-terminal
tool calls. The Worker supersede update then clears `active_tool_call_id` and
`active_tool_started_at` together with its lease fields.

This ordering preserves the active-tool fence while the tool row is terminalized and
prevents a late tool completion from rewriting the new reset epoch.

### 5.3 Atomicity

Old tool terminalization, Worker/Unit supersede, handoff invalidation, old execution
close, replacement execution/Unit creation, cursor update and V2 reset marker remain in
one database transaction. Any constraint failure rolls all of them back.

The optional fact purge intentionally remains a following transaction because it is an
explicit destructive mode with its own all-or-nothing domain-fact cleanup contract.

## 6. Failure Semantics

- A mismatched active tool/Worker owner tuple fails the reset transaction; it is not
  silently ignored.
- Already-terminal tools are retained unchanged.
- A replacement execution that contains any Worker before Team seed is a regression.
- Reset never marks an unknown tool successful and never creates checked-empty evidence.

## 7. Verification

- RED/GREEN integration: reset a V2-only Vuln Company stage, assert zero replacement
  Workers before seed, then seed Team runtime and claim the Controller.
- RED/GREEN integration: attach a running tool to an old Worker, reset, and assert the
  tool is failed with the structured reset outcome while the Worker is superseded with
  a null active-tool pointer.
- Focused `golish-db` nextest for the named reset tests.
- Scoped `cargo clippy -p golish-db --all-targets -- -D warnings`, rustfmt check, JSON/
  Markdown/diff checks. No full-workspace gate unless separately authorized.
