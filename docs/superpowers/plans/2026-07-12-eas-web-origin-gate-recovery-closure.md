# EAS Web-Origin Gate and Recovery Closure Implementation Plan

## Goal

Make EAS WEB preflight, repair execution, and continuation semantics converge on
the same DB-backed exact-origin truth.

## Tasks

1. Add a RED `golish-agent-kit` regression proving asset-level EAS WEB terminal
   exceptions cannot make an origin-incomplete snapshot ready, while another
   EAS technique can still use the supported exception path.
2. Add RED `golish-sub-agents` repair regressions for exact worklist objects,
   equivalent exact strings, and rejected scheme/port rewrites; implement the
   smallest deterministic authorization projection.
3. Add a RED runtime/orchestrator regression for retry-exhausted automatic
   continuation and preserve fresh-budget behavior for a later explicit user
   request.
4. Close repair-lock liveness: apply a DB refresh to later calls in the same
   batch, persist it in `agent_run`, preserve it through stage retry, and refuse
   to clear it from a bounded empty page without explicit readiness.
5. Run focused suites for every changed crate, then scoped Clippy, rustfmt, JSON,
   and diff checks.
6. Update affected module cards, `agent-progress.md`, and the current sole
   `in_progress` feature. Keep it in progress until a freshly compiled live EAS
   continuation closes the exact-origin denominator and receives a pass token.

## Non-goals

- Do not accept model-authored WEB `found`/`blocked` as exact-origin truth.
- Do not infer HTTP or HTTPS from a numeric port.
- Do not reset a retry budget inside the same top-level user request.
- Do not delete or rewrite prior run evidence.

## Focused verification status

- RED reproduced EAS WEB parent exceptions making preflight ready, then a
  second RED locked the rejected-entry contract. GREEN keeps the exact-origin
  gap actionable and preserves other EAS terminal exceptions.
- RED reproduced host-level repair authorizing a rewritten scheme. GREEN binds
  WEB repair to the refreshed DB worklist's exact target-id/origin pairs and
  emits exact-origin recovery actions for partial-only tail gaps.
- RED reproduced automatic reflector execution after stage-run exhaustion.
  GREEN stops same-request retries while retaining fresh-budget behavior for a
  later explicit user request.
- Repair-lock liveness and stale-action regressions passed 5/5 (run
  `fd41b4cf-9e90-4bf0-84c3-833c349c1bb1`); the no-monitor DB observer path and
  durable checkpoint projection passed 3/3 (run
  `8ff63e59-321f-4035-bb80-1b72dec6588d`). Full sub-agent/runtime passed
  521/521 (run `42b0c009-8c3d-4e7f-967e-98c6ccb635e8`). The refined exact lock
  now applies within the same batch, survives checkpoint restore and stage
  retry only for the same WEB action identity, and cannot be erased by a
  bounded empty projection.
- Independent combined focused run `9f5ca240-70bc-46ef-93e0-218b1fdfbd84`
  passed 9/9 across `golish-agent-kit`, `golish-agent-bridge`,
  `golish-sub-agents`, and `golish-agent-runtime`.
- Final four-crate suite passed 1488/1488 (run
  `c5fbea93-4014-4dc6-8982-98e92c74d76a`) and scoped all-target Clippy passed
  with `-D warnings`.
- Fresh live continuation at 15:06 proved the prior fixes were loaded and
  reduced the exact-origin gap from 188 to 80, but exposed one more state seam:
  an intervening repeated `needs_fix` replaced the exact lock with host-level
  mode. RED run `f30864b5-6ebd-4e03-856d-bc23f5af0eda` captured `Some([...]) ->
  None`. GREEN now shares same-gap retention across executor, runtime checkpoint,
  and stage retry; focused 4/4 passed in run
  `8e3bbfba-a0bd-48fb-b6b6-4de0e790c447`, sub-agent/runtime passed 524/524 in
  `39ba57bd-5e8d-4672-b2f1-be7358ec0280`, and the final four-crate suite passed
  1489/1489 in `47310cec-b937-404e-8a27-7a8b57e5209f`.
- Fresh compiled live EAS continuation remains pending.
