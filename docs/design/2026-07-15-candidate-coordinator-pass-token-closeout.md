# Candidate coordinator pass-token closeout

## Context

`attack_candidate` is a specialist fan-out stage. Each organization worker owns an exact
`StageRunUnit`, persists its Candidate decisions, and finalizes that unit. The top-level coordinator
then submits only the deterministic `stage_run_pass_token`; it is not another worker and therefore
must not claim a unit or worker lease.

The 2026-07-15 CLI acceptance reached a fully passed `attack_analyst` unit, but aggregate closeout
returned `attack_candidate submit preview requires StageRunUnit identity`. The submit tool correctly
normalized the coordinator token without writing a second per-unit submission, then incorrectly built
Candidate's unit-scoped manifest preview before taking the aggregate pass-token fast path.

## Contract

1. A trusted Main-context coordinator closeout is recognized only when it contains the unique
   specialist pass token and has no `StageRunUnit`, worker lease, or Candidate attempt identity.
2. That aggregate receipt is normalized and captured for the final orchestrator gate without creating
   another per-unit durable submission.
3. It must return before any unit-scoped Candidate manifest preview is constructed.
4. Worker submissions and ordinary post-Scoping submissions keep their exact unit, organization, and
   worker-lease fences.
5. The final fan-out gate remains authoritative: it re-derives the token from current-operation
   `org_stage_completions`; accepting the submit preview does not itself publish stage PASS.

## Validation

- RED: an Attack Candidate coordinator token with trusted operation/execution context but no unit is
  rejected by the old preview ordering.
- GREEN: the same call is accepted, writes no second per-unit submission, and stores only the
  normalized aggregate token.
- Existing Target Intel coordinator and missing-unit worker-fence tests remain green.
