# Candidate terminal slice and evidence semantics

## Context

The fresh CLI acceptance run
`stage-run-054467a0-e5a0-4b88-ac3e-57b386153772` reached Attack Candidate and
returned exit code 0. The Candidate specialist Unit passed, the coordinator
submission was accepted, and the final-sealed Candidate handoff was persisted.
The run also exposed two terminal-slice edge cases and one reasoning-quality
gap.

## Contract

1. Candidate review is a crossing barrier for `attack_candidate -> verification`.
   A projected DAG that ends at Candidate has no crossing, so it must not read
   or hold on the V2 review barrier.
2. `blocked` is an exact evidence outcome meaning the producer did not complete
   its check. It is not a negative result and does not identify WAF, rate
   limiting, target resistance, or any other cause unless trusted evidence
   explicitly carries that cause.
3. A completed terminal graph must close its exact active `stage_runs` row
   without creating a successor or moving the operation cursor. The close must
   be an operation/execution/current-stage checked transaction and must happen
   atomically with the task terminal write, or fail closed.
4. Stopping at Candidate deliberately does not perform Candidate review,
   Verification, or wave consolidation. A dual-write V2 mirror may therefore
   remain `review` / `consolidation_status=pending`; that is not a Candidate
   Gate failure.

## Changes allowed without DB-layer authorization

- Resolve the projected successor before consulting the Candidate review
  barrier. `None` returns Allowed immediately.
- Add shared Candidate methodology and `list_recent_evidence` contract language
  forbidding invented blocker causes.
- Add focused regression tests for both behaviors.

## Deferred DB transaction

The terminal `stage_runs` close belongs in the `golish-db` transaction layer so
the operation row, exact active execution, current-stage cursor, task terminal
state, and completion timestamp cannot diverge. Repository policy requires
explicit user confirmation before changing `golish-db`; until that approval is
given, the live run evidence must be reported honestly as:

- Candidate Unit and final-sealed handoff: complete;
- top-level task and graph: finished / `__end__`;
- exact terminal `stage_runs` row: incorrectly left `started` (known bug);
- Candidate wave review/consolidation: intentionally not run.

## Verification

- RED/GREEN: `v2_only_terminal_attack_candidate_slice_never_reads_review_barrier`
- focused semantics: `candidate_reasoning_never_invents_a_blocker_cause`
- retain the existing full-DAG review test so V2Only Candidate still holds
  before a real Verification crossing.
