# Resume normalization and runtime decoupling

> **Status:** Proposed on 2026-08-14. This document is planning-only. It does not authorize a
> database migration, deletion of compatibility code, or mutation of a retained operation.
>
> **Relationship to earlier designs:** preserves the Operation Thread / Turn CAS contract from
> `2026-07-16-codex-style-agent-thread-resume.md` and the bounded durable-continuation contract from
> `2026-08-10-codex-style-durable-continuation.md`. Once implemented, it supersedes only their
> scattered resume selection, repair and dispatch wiring. It does not weaken project ownership,
> whole-record source pinning, worker/chain identity, active-tool fences or append-only evidence.

## 1. Problem statement

Golish currently has the right low-level safety ingredients, but they are assembled in several
different places and at different abstraction levels. A user-visible `继续` may pass through GUI,
Candidate Review or CLI wiring, while recoverability is independently interpreted by task SQL,
CLI relational loading, startup reaping and `stage_run` runtime control.

The result is not merely “large files”. It is a split authority problem:

- GUI resume preflight and execution each query the candidate separately. A database lookup error in
  `chat.rs` is currently converted to `None`, allowing a fresh operation to start instead of failing
  closed.
- `operation_resume.rs` reads the open Turn, runtime source and bound chains in separate autocommit
  reads. The later Turn CAS prevents two successful claims, but selection is not one MVCC snapshot.
- exact resume is represented by three independently mutable facts: orchestrator runtime source,
  Bridge runtime source and `resume_task_preclaimed`. GUI, Candidate Review and CLI reproduce the
  required call order.
- startup reaper and ordinary resume do not share one canonical authority predicate. In particular,
  post-synthesis relational recovery is admitted by the reaper predicate but is not present in the
  ordinary latest-candidate predicate.
- CLI owns a useful relational resume classifier, but GUI does not use it. Its loader mixes queries,
  identity checks, message decode and classification, then flattens distinct repair decisions into a
  generic authority object.
- `stage_run_call.rs` and `runtime_memory_tx.rs` each exceed 25,000 lines and interleave snapshot,
  classification, repair, provider dispatch, compatibility and response projection.
- deterministic blockers are frequently represented as strings and correlated JSON fields. The
  request-local `StageRunReentryGuard` stores only `StageKind`, resets on every user request, and
  therefore cannot recognize the same unchanged blocker after another `继续`.
- `TaskOrchestrator::resume` marks every escaped error as terminal task failure. A deterministic
  pre-provider mismatch can therefore convert a recoverable operation into `failed`.
- tracked tool state and the worker `active_tool` fence begin and finish in separate transactions.
  A crash can leave a clean worker with a still-running tool row, or another partially visible tuple.

Repeated `继续` therefore re-enters a distributed decision process rather than advancing one closed,
versioned state machine. Adding another local compatibility branch fixes a fixture but increases the
next resume's state space.

## 2. Design goals

1. One authoritative pipeline decides whether an operation is fresh, resumable, safely repairable,
   busy, operator-blocked, unsupported, terminal or corrupt.
2. GUI, Candidate Review, CLI and startup recovery reuse the same snapshot and classification
   semantics while retaining their adapter-specific authorization and barrier CAS.
3. Only a `Ready` decision may advance the Operation Turn or dispatch a provider.
4. Safe internal repair runs to a bounded fixed point before a new top-level Turn is claimed.
5. The same durable state produces the same typed blocker and fingerprint across user requests.
6. External outcome-unknown state is never automatically replayed.
7. A behavior protocol version is frozen on every stage execution. Storage rollout version and graph
   topology are not used as substitutes for behavior compatibility.
8. Historical rows, evidence, migrations and safety readers remain readable after old execution
   compatibility is retired.
9. File extraction follows stable behavioral seams; it is not a simultaneous rewrite of the agent
   loop or database runtime.

## 3. Non-goals

- Do not replace the Golish agent loop with Claude Agent SDK or another provider SDK. Provider-native
  sessions remain an adapter optimization, not the canonical resume authority.
- Do not resume the currently failed retained Investigation operation by weakening a fence.
- Do not reinterpret old `fixed_roster_v1` or unversioned stage rows as the current dynamic Primary
  protocol.
- Do not remove old migrations, append-only receipts or history projections.
- Do not redesign every Stage Team scheduler in the first slice.
- Do not perform external target I/O inside resume normalization or a database transaction.

## 4. Three different contracts

The platform must stop treating the following identifiers as interchangeable:

| Contract | Answers | Current example | Resume use |
|---|---|---|---|
| runtime-memory storage contract | Which physical record format is authoritative? | `v2_only` | whole-record source selection only |
| stage topology contract | Which stages and transitions exist? | `unified_investigation_v1` | graph identity and legal-stage validation |
| stage resume contract | Which behavior, repair rules and scheduler invariants created this active stage execution? | `investigation_asset_primary_dynamic_v2` | first resume compatibility gate |

`unified_investigation_v1` remained unchanged while Investigation moved through several internal
schedulers. A new `stage_runs.resume_contract` is therefore required. It is immutable for the
execution and checked before any stage-specific read, repair or provider dispatch.

The first forward migration assigns existing rows `legacy_unversioned_v1`; a DB trigger rejects that
value on every later insert, so new executions cannot silently remain unversioned. New code writes
one of the explicitly supported contracts. An unfinished legacy/unversioned execution returns
`UNSUPPORTED_FROZEN_CONTRACT` with a rerun/fork action; it is never silently interpreted using the
new scheduler. Completed rows remain ordinary audit history.

## 5. Canonical state machine

```text
Find candidate
  ├─ none ----------------------------------------------> FreshAllowed
  ├─ storage/read error --------------------------------> InfrastructureError
  └─ candidate
       └─ Select one repeatable-read ResumeSnapshot
            └─ Classify (pure)
                 ├─ Terminal ---------------------------> replay terminal view
                 ├─ Busy -------------------------------> no claim / no provider
                 ├─ AwaitOperator ----------------------> no claim / no replay
                 ├─ UnsupportedContract ----------------> RERUN_REQUIRED
                 ├─ Corrupt ----------------------------> fail closed
                 ├─ SafeRepair(plan) -------------------> CAS repair, reload snapshot
                 │                                         (bounded fixed point)
                 └─ Ready(directive)
                      └─ claim snapshot fingerprint + open Turn in one tx
                           └─ ClaimedExactResume ticket
                                └─ dispatch exactly once
```

The public decision is closed:

```rust
pub enum ResumeDisposition {
    Ready(ResumeDirective),
    SafeRepair(Vec<ResumeRepair>),
    Busy(ResumeBlocker),
    AwaitOperator(ResumeBlocker),
    UnsupportedContract(ResumeContractMismatch),
    Terminal(ResumeTerminal),
    Corrupt(ResumeBlocker),
}
```

An infrastructure error is a function error, not `None`, `FreshAllowed`, `Corrupt` or a model-visible
retry opportunity.

## 6. ResumeSnapshot

`select_resume_snapshot` uses one `REPEATABLE READ READ ONLY` transaction. It reads only the durable
authority needed to classify and claim:

- operation/task/session/project/profile and whole-record source;
- immutable stage resume contract and active stage execution;
- the one open Operation Turn;
- current scope and active Unit/Plan/WorkItem/WorkerRun identities with row/attempt/checkpoint epochs;
- bound message-chain identities plus content hashes; body JSON is returned for later typed decode;
- lease and active-tool tuples;
- stage-specific authority references required by the explicit classifier, including current
  Investigation company/asset/Primary lineage or Company Controller gate material;
- a canonical snapshot fingerprint.

Display strings, unrestricted tool output, secrets and whole legacy state blobs are excluded. The
fingerprint hashes stable identifiers, statuses, versions, source, resume contract, tool fence and
chain-body hashes. It never hashes free-form error detail or secret-bearing output.

The first implementation uses explicit `match stage_kind` modules. It does not introduce a dynamic
plugin registry.

## 7. Classification and safe repair

Classification is a pure function over `ResumeSnapshot`. It performs no SQL, model call, filesystem
mutation or external I/O. Stage-specific classifiers produce only typed directives and repair plans.

Safe repair is a closed allowlist. A repair must be:

- deterministic from immutable source identity;
- idempotent and receipt-backed where it creates a successor;
- protected by exact IDs, status, row version, attempt/checkpoint epoch, lease/tool fences and chain
  hash;
- free of provider or target I/O;
- followed by a fresh snapshot and reclassification.

The normalizer allows at most eight successful state transitions per invocation. Repeating the same
snapshot fingerprint and repair identity, or applying a repair without changing the classified
state, returns `RESUME_REPAIR_STALLED`. It does not open a Turn or dispatch a provider.

Examples of safe repair include requeueing an expired worker with no active tool and applying an
already-defined receipt-backed Controller successor. An active external tool, missing result after
dispatch, or inconsistent tool/worker tuple is `AwaitOperator` or `Corrupt`; it is never `SafeRepair`.

## 8. Exact claim ticket

The claim transaction locks in this order:

1. operation/task and open Turn;
2. active stage execution and scope;
3. Unit, Plan, WorkItem and WorkerRun authority;
4. tool/chain/receipt fences.

It recomputes the snapshot fingerprint, verifies the selected whole-record source and resume
contract, closes the exact prior Turn, inserts one successor Turn, marks the task running and applies
existing successor-Turn Controller effects in one commit.

Success returns an opaque `ClaimedExactResume` ticket containing operation/session identity, source,
prior/successor Turn IDs, resume contract, snapshot fingerprint and dispatch directive. Only the DB
claim module can construct it. `TaskOrchestrator::resume_claimed` consumes it once. This removes the
public production path where callers separately set source, Bridge source and a preclaimed boolean.

Candidate Review keeps its `resume_pending -> dispatching` barrier CAS outside the generic
coordinator. CLI keeps its advisory lock and explicit operator-repair UX. Both then call the same
coordinator and consume the same ticket.

## 9. Runtime control and blocker identity

`stage_run` returns a single typed `StageRunRuntimeControl` projection:

```rust
pub struct StageRunRuntimeControl {
    pub kind: RuntimeControlKind,
    pub reason: StageRunHaltReason,
    pub retry: RetryDisposition,
    pub blocker_fingerprint: String,
    pub blockers: Vec<ResumeBlocker>,
}
```

`tool_dispatch.rs` deserializes this object. It no longer reconstructs meaning by correlating
`reason`, `passed`, `retry_budget_exhausted`, scheduler strings and gap arrays. A halt can require a
new request without claiming retry fuel is exhausted.

The request-local reentry guard stores a typed halt token keyed by stage and blocker fingerprint.
Across requests, the preflight snapshot classifier independently reproduces the same fingerprint and
stops before provider dispatch. The guard remains a circuit breaker, not durable authority.

Repository mismatches, human-input holds, provider failures and retry exhaustion retain distinct
codes. A deterministic deferred outcome leaves the task waiting with `result = NULL`; only a true
terminal execution failure marks the task failed.

## 10. Tool fence atomicity

Resume correctness depends on being able to distinguish safe retry from outcome unknown. The tracked
tool row and WorkerRun fence therefore change together:

- `begin_tracked_worker_tool`: insert/claim the tool row and CAS the worker active-tool tuple in one
  short transaction; commit before external dispatch.
- `finish_tracked_worker_tool`: persist terminal tool outcome and clear the exact worker tuple in one
  short transaction.
- duplicate begin/finish replays return the already-committed canonical result; mismatched attempts
  fail closed.

No external command, HTTP request, model call or message queue operation occurs inside either
transaction.

## 11. Module ownership

### Shared domain

`golish-core/src/resume.rs` owns stable source/protocol IDs, blocker/error codes, retry disposition,
runtime-control serialization and canonical fingerprint helpers. It has no SQL or provider types.

### Database authority

`golish-db/src/repo/runtime_resume/` owns:

- `authority.rs`: the one candidate/source/recoverability predicate;
- `snapshot.rs`: repeatable-read authority projection and fingerprint;
- `claim.rs`: open-Turn/fingerprint CAS and opaque ticket;
- `repair.rs`: closed, idempotent DB repair actions;
- `reaper.rs`: startup reconciliation using the same authority and repair seam;
- `tool_fence.rs`: atomic tracked-tool/WorkerRun lifecycle.

`tasks.rs`, `message_chains.rs`, `tool_calls.rs` and `runtime_memory_tx.rs` retain compatibility
facades during extraction, but do not remain independent resume authorities.

### Application coordinator

`golish-agent-app/src/ai/resume/` owns:

- pure common and stage-specific classification;
- the bounded snapshot -> classify -> repair loop;
- project/scope/chain decode validation;
- the coordinator API used by GUI, Candidate Review and CLI.

### Orchestrator and runtime

`golish-agent-kit` consumes the opaque claim ticket and removes generic unfenced production resume.
`golish-agent-runtime/src/agentic_loop/tool_execution/direct/stage_run/` separates snapshot,
classification, repair, dispatch and control projection. `stage_team_scheduler.rs` keeps pure plan,
worklist, hashing and bounded output contracts.

### CLI

`golish/src/stage_run/runtime_v2.rs` stops owning a second canonical resume classifier. CLI-specific
reporting, advisory locks, workspace checks and operator commands stay in the binary adapter.

## 12. Compatibility retirement policy

Compatibility is retired by behavior, not by deleting every historical symbol at once.

### Preserve permanently or until an archival replacement exists

- append-only operation, Turn, stage, worker, tool, chain and repair receipts;
- old migrations and checksum history;
- read-only history/report projections;
- project/scope/identity/lease/tool/chain safety fences;
- whole-record source pin and open-Turn CAS.

### Retire after the new gate is live

- old writers and schedulers for unsupported stage resume contracts;
- fixed-roster runtime selection and cutover as runnable authority;
- generic unfenced `TaskOrchestrator::resume`;
- duplicated GUI/CLI/Candidate select/set/claim/set wiring;
- string/correlated-JSON halt inference.

### Separate later retirement

Global `LegacyV1`, dual-write and `LegacyFallback` storage-source removal is not bundled with the
Investigation behavior cutover. It requires a fresh database census proving there are no runnable
operations on those contracts, at least one published compatibility window, and explicit approval.
Until then, legacy source policy remains isolated behind `authority.rs` and cannot leak into the
canonical V2 ticket.

## 13. Rollout order

1. Add characterization tests for current invariants and known fail-open/failure-finalization bugs.
2. Add the immutable stage resume contract and reject unversioned unfinished executions before
   provider dispatch.
3. Introduce shared snapshot, pure classifier, bounded repair loop and atomic claim ticket while
   preserving current legacy source policy.
4. Route GUI, Candidate Review and CLI through the coordinator.
5. Make tool lifecycle atomic and introduce typed runtime control/finalization.
6. Extract large files along the now-tested seams without changing behavior.
7. Run a census, then delete unsupported Investigation runtime compatibility with explicit approval.
8. Treat global Legacy/dual storage-source retirement as a later, independent migration.

The active Investigation closure must finish or be deliberately abandoned before step 2 is applied
to the shared development database. The migration intentionally makes existing unversioned active
stage executions non-runnable.

## 14. Acceptance criteria

1. A DB error during resume discovery never starts a fresh operation.
2. GUI, Candidate Review and CLI produce the same disposition for the same snapshot.
3. snapshot selection is one repeatable-read view; a drifted fingerprint makes claim zero-write.
4. two contenders can create at most one successor Turn.
5. only a consumed `ClaimedExactResume` ticket can enter the production resume path.
6. unsupported or corrupt state opens no Turn, changes no task status and dispatches no provider.
7. safe repairs converge to `Ready` or one stable typed blocker within the fixed transition bound.
8. outcome-unknown tool state always requires explicit recovery and is never replayed.
9. reaper-preserved post-synthesis state is also discoverable by ordinary resume.
10. deterministic blockers remain waiting, not failed, and reproduce the same fingerprint after a
    second `继续` without another provider dispatch.
11. fresh current-contract Investigation resumes the same Asset Primary chain without duplicate
    tool execution.
12. historical rows and reports remain readable after old runtime writers/schedulers are retired.
