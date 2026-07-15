# Company-scoped stage legacy runtime retirement

> Status: implemented and localhost CLI accepted; no DB schema, migration, or rollout row was changed.
> Safety checkpoint before any legacy deletion: commit `5af4f31a`.

## Problem

Target Intel, External Attack Surface, Enumeration and Vuln Triage now have the same StageSpec and
Company Controller implementation, but the product still contains a runtime compatibility fork:

- `RuntimeMemoryContract::V2Only` seeds `stage_team_plans` and runs `company_controller_v1`.
- `DualWriteLegacyRead`, `DualWriteV2Preferred`, `LegacyV1`, or a missing runtime-memory authority
  can still fall through to the old per-organization specialist loop.
- `StageRunOrgRows` renders the DB-backed Team view only when every row has an exact operation and
  stage-execution pointer; otherwise it still renders `Main Agent -> Specialist` cards.

The fork is not presentation-only. Stage Team repository mutations (`seed`, recovery, worker output,
retry, Controller Gate repair and finalization) deliberately reject operations whose frozen contract
is not `v2_only`. Lifting the runtime `if V2Only` check without changing the DB contract would therefore
fail after dispatch and would violate the atomic rollout model.

## Decision

Retire the old company-stage path only when the existing attested Runtime Memory rollout already selects
`v2_only` for new operations:

1. Preserve the already-created safety checkpoint `5af4f31a`.
2. Read the deployment contract and its promotion receipts before changing runtime behavior. The current
   deployment was already at `v2_only`, so this implementation made no DB/rollout mutation. If another
   deployment is not at `v2_only`, its existing adjacent, cohort-attested promotion remains a separate
   operator action; never update frozen historical operation contracts or bypass the rollout trigger.
3. New company-stage operations then always enter the durable Company Controller scheduler.
4. Existing `legacy_v1` / dual-write company-stage executions do not silently run the old specialist
   scheduler. They return a typed rerun-required error and remain inspectable as historical data.
5. Remove the old `StageRunOrgRows` collector cards, coverage chips and `Main Agent -> Specialist`
   summary. Rows without one exact Team pointer render only the rerun-required terminal notice.
6. Add a runtime guard immediately before the generic specialist loop so the four company stages can
   never regress into it, even if an earlier scheduler selection is changed later.

Candidate and Verification are not part of this deletion. Their Wave/CandidateAttempt schedulers and
rollout contracts remain separate. Post-Exploit, Reporting and Cleanup also retain their typed paths.

## Database and recovery boundary

The deletion itself does not change deployment/runtime authority or touch `golish-db`. The implementation
first verified that the current deployment singleton was already `v2_only` (rank 3, row version 3), then
limited code changes to runtime selection/resume, frontend routing and tests. A future deployment promotion
still changes DB rollout authority and remains a separately approved high-risk action under `AGENTS.md`.

- Existing operation contracts remain immutable.
- Promotion must use the current cohort gate and promotion receipts.
- If the current deployment lacks a valid shadow cohort, promotion must stop with the exact gate reason;
  no migration may disable or rewrite the attestation trigger.
- Historical legacy/dual operations become rerun-required for these four stages; they are not rewritten
  into fake Team rows.
- No evidence, scope, Gate or authorization rule is relaxed.

## Acceptance

1. A source search finds no company-stage path that invokes the generic per-org specialist scheduler.
2. Target Intel, EAS, Enumeration and Vuln all require exact Team runtime authority and return the same
   typed rerun-required error for historical non-V2 operations.
3. `StageRunOrgRows` has no `Main Agent`, legacy collector card, coverage-chip or specialist drill-in
   implementation.
4. Candidate/Verification and later typed schedulers retain their existing tests and behavior.
5. Existing rollout attestation tests pass; a fresh/local deployment can reach `v2_only` only through a
   valid adjacent promotion.
6. A fresh localhost CLI slice shows `company_controller_v1` for EAS, Enumeration and Vuln, with no
   legacy UI/runtime signature.

## Implemented result

- Company stages now fail closed with `STAGE_TEAM_POLICY_REQUIRED`,
  `STAGE_TEAM_V2_RERUN_REQUIRED`, or `STAGE_TEAM_ROUTE_INVARIANT` before the generic specialist loop.
- A completed Company Controller stage replays its operation-fresh aggregate pass token without reseeding
  Team rows or dispatching a provider.
- Exact CLI resume validates the Team Plan and every WorkItem/Worker identity, selects the unique
  `leader:primary` Controller as Unit owner, and permits only correctly bound dynamic child Workers.
- `StageRunOrgRows` only mounts the DB-backed Team view for the four company stages; a missing/mixed exact
  pointer shows the rerun-required notice, while Candidate/Verification and later stages keep typed views.
- Localhost acceptance session `stage-run-c6331c37-48e9-4ea1-a93c-e2082762c72d` exited 0. EAS,
  Enumeration and Vuln each final-sealed under `company_controller_v1`; run-tree DB truth selected
  `v2` with legacy fallback forbidden and showed three passed `leader:primary` Controllers.
