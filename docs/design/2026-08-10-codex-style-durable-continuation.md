# Codex-style durable continuation

## Problem

Golish currently preserves the Stage Team graph, WorkerRun, message chain and
tool-call row, but it does not preserve one uniform continuation decision. A
restart can therefore collapse several materially different states into
`recovery_required`:

- a bounded read such as `check_job` was interrupted;
- a monotonic stage producer was interrupted after durable partial progress;
- an in-memory managed process disappeared with the application;
- a side-effecting action has an outcome that cannot be reconstructed.

The first three states can be resolved by the host. Requiring another user
message for each one makes continuation depend on repeated trial and error.

## Decision

Continuation is a server-owned state machine. The model may explain or select
new work, but it does not decide whether an interrupted action is safe to
resume.

The closed recovery classes are:

1. `refresh_read`: local, bounded, read-only observations (`check_job`,
   `wait_for_background_jobs`). The old call is terminalized as interrupted and
   the same WorkerRun/message chain is requeued. The old arguments are never
   replayed.
2. `resume_from_durable_truth`: exact-scope monotonic producers whose business
   progress is already represented by current coverage/evidence. The same
   worker reloads the worklist and executes only remaining cells.
3. `producer_budget_exhausted`: a server-owned producer admission/deadline was
   consumed without complete terminal evidence. The host records guarded
   `blocked` evidence and closes the producer cell; the model cannot relaunch it
   by changing prose, input order, or transport-yield arguments.
4. `outcome_unknown`: an action may have produced an unobservable external side
   effect. It remains fail-closed and requires an exact operator decision.

The Company Controller must attempt one exact safe child reconciliation before
its recovery barrier is evaluated. A safe child must therefore never be hidden
behind an earlier aggregate `operator_recovery_required` result.

## Request behavior

One user continuation opens one durable Turn and should drain every currently
safe recovery transition. Internal refresh/requeue transitions do not consume a
new user Turn. A request stops only for:

- exact authorization/review input;
- an outcome-unknown side effect;
- a deterministic Gate blocker that requires genuinely new work;
- an infrastructure failure for which no typed recovery class exists.

Server-authored retry and producer budgets remain bounded. "Automatic" means
the host performs already-authorized state transitions; it never means an
unbounded loop.

## Managed network producers

Generic Codex-style shell jobs retain bounded-yield/no-wall-clock-kill
semantics. A security producer may additionally install an immutable,
server-owned policy deadline. This is not a model `timeout_secs` and does not
apply to arbitrary shell commands.

EAS full-port discovery freezes one admission key from:

- operation id and EAS epoch;
- organization id;
- sorted exact target ids;
- fixed scanner/profile version;
- expanded-host manifest hash.

The admission is written before network launch in the reserved
`operation_state` namespace. For the initial contract, the exact full-profile
manifest receives one launch. Natural success closes coverage normally. A
server deadline, application/process loss, cancellation, spawn failure or
incomplete output consumes that launch; a later invocation emits guarded
`EAS_PORT_SCAN_ATTEMPTS_EXHAUSTED` evidence and terminal `blocked` LIVENESS/PORT
outcomes without launching the network again.

This intentionally prefers an explicit residual over repeatedly scanning the
same real asset. A future contract may permit more than one attempt only by
versioning the server policy and retaining the same exact admission identity.

## Process-loss boundary

In-memory `job_id` values are not durable OS handles. After process loss Golish
does not pretend that `check_job` can observe the old job. Durable truth is the
tool call, WorkerRun, producer admission and coverage/evidence state. A missing
job is reconciled through those records. Any orphan process discovered during
development or startup must be stopped before another admission can launch;
the product must not run two copies of the same frozen producer manifest.

## Acceptance

- Controller claim reconciles an interrupted safe child before barrier read.
- The same WorkerRun and message chain are reused; old tool arguments are not
  replayed.
- `kill_job` and side-effecting tools are not classified as refresh reads.
- A full-port producer has a trusted policy deadline and exact prelaunch
  admission.
- A repeated call for the same frozen manifest launches zero network and lands
  guarded blocked outcomes.
- Malformed/foreign/stale operation-state slots fail closed.
- A retained real operation can continue without repeated user messages until
  it reaches a genuine authorization/Gate boundary.

## Unified Investigation retained-child continuation

The 2026-08-11 retained Moresec run exposed a separate restart seam after the
Analysis Primary had already accepted and persisted a dynamic child. The
static Stage Team seed deliberately contains only server-seeded items, so a
restart must not infer that the accepted child disappeared merely because it
is absent from the seed response.

The recovery contract is therefore exact and additive:

- the host reloads the persisted WorkItem by its durable id and verifies the
  same operation, StageExecution, Unit, scope, organization and TeamPlan;
- the accepted dispatch receipt and latest Refiner patch are reloaded from the
  same authority instead of rebuilt from current mutable context;
- a Primary parked at `waiting_dependency` can be returned read-only only when
  exactly one pending child is attached to that Primary; no fake lease or new
  message chain is created;
- the scheduler treats that witnessed Primary as planning-only and resumes the
  exact child identity, including deterministic sibling retry after a failed
  Worker;
- Main read-session replay uses the sealed ContextPack/methodology census and
  domain-separated Refiner payload hash. It never re-runs mutable RAG reads to
  impersonate the original session.

This contract was exercised by operation
`4d5f17a5-88f5-423e-9dcb-3e9cad6e1003`: five Analysis children completed,
four Verification Campaigns reached terminal state, one missing
`submit_result` was repaired on the same chain, one failed Worker converged via
an exact sibling retry, and the Main stage retry reached `pass_with_gaps`
without another user continuation. Reporting then created validated concise
revision `287cbf51-aec7-5254-936c-4029d18c9d31` and the task became
`finished`.
