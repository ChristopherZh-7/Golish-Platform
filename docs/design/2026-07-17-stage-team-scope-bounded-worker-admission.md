# Stage Team scope-bounded worker admission

## Background

The live Vuln Triage run for operation `2b2c2271-b8ea-4196-897b-799f144bb9ee` had an authoritative
worklist of 21 exact origins and 210 origin × technique cells. The Company Controller successfully
dispatched eight one-origin Vuln Scanner children. Its ninth and tenth valid requests were then
persisted as rejected with `stage_team_dynamic_request_limit_reached`, solely because the StageSpec
froze `max_dynamic_requests=8`. The Controller fell back to doing the remaining 13 origins itself.

This is not a concurrency failure. The three live limits already have separate meanings:

- C: concurrently active company Units;
- G: concurrent provider calls across the stage run;
- K: live WorkerRuns inside one company Unit, including the Controller.

`max_dynamic_requests` and the derived `max_workers_total` added a fourth, lifetime-wide quota that
was unrelated to scope or the deterministic worklist. A larger legitimate company therefore failed
where a smaller company passed, even though both obeyed the same concurrency and authorization
rules.

## Decision

Company Controller plans no longer use a lifetime worker/request total as admission authority.
The number of durable children may be 0..N and is driven by the Controller's current, server-visible
worklist and Gate gaps. Admission remains fail-closed on:

1. exact Controller Worker fence and open dispatch epoch;
2. frozen operation/stage/unit/organization/project scope;
3. canonical subject refs and server role/kind allowlists;
4. deterministic request replay by dedupe key and payload hash;
5. per-dispatch batch bound;
6. C/G/K live concurrency;
7. each WorkItem's bounded attempt policy;
8. cancellation, lease loss, recovery-required tool state, and deterministic Gate truth.

The DB's existing `max_workers_total` column cannot be removed without a migration. The frozen
StageSpec/TeamPlan also participates in exact restart replay, so removing `max_dynamic_requests` or
rewriting an already-seeded plan would strand active operations with a plan-hash mismatch. This change
therefore keeps both historical values as compatibility metadata, but Company Controller request
admission, fresh Worker claim, retry scheduling, and scheduler loops do not consult them.
Non-Controller legacy Team plans retain their existing lifetime-cap behavior.

## Runtime behavior

### Dynamic request admission

`coordination_mode=company_controller` is the durable discriminator. For such plans,
`request_stage_worker` ignores historical `max_requests` and `max_workers_total` values, including on
already-created plans. It still persists every decision, rejects stale/closed/foreign requests, and
replays an existing dedupe key exactly.

Previously rejected request rows remain immutable audit history. A continued Controller may submit a
new corrected logical assignment with a new dedupe key; the repo must not rewrite an old rejected row.

### Claim and retry

Fresh Company Controller children are constrained by `max_workers_active`, not by the count of all
WorkerRuns ever created. Once a live child terminalizes and frees a slot, another queued WorkItem can
be claimed even if the compatibility total has been exceeded. A failed child retries only while its
own `attempt_policy.max_attempts` has fuel.

### Controller scheduler

The child-drain and Controller coordination loops terminate on durable queue/barrier/finalization
state rather than `max_workers_total`. Each dispatch tool call is still limited to 32 requests, and
parking the Controller returns provider capacity before children run.

## UI

The Stage Team detail view shows `K active workers max` and no longer displays `active / total`. The latter
advertised a limit that is not valid for Company Controller plans. The compatibility field remains in
the read DTO until a separately authorized schema/API cleanup.

## Safety and non-goals

- This does not increase C/G/K concurrency.
- This does not loosen target, organization, project, role, kind, evidence, or Gate checks.
- This does not let children dispatch grandchildren or submit the Unit deliverable.
- This does not change Candidate/Verification or other typed schedulers.
- This does not alter DB schema, migrations, generated IPC types, or frozen plan replay material.
- The deterministic worklist and Gate remain completion authority; child count is never completion
  evidence.
