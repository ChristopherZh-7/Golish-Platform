# EAS Web-Origin Gate and Recovery Closure

Status: Implemented with focused verification; fresh compiled EAS continuation remains pending.

## Live failure

The explicit user continuation in `pentest-chat-1783829178322-1` did resume the
saved EAS Prober and persist additional WhatWeb evidence. It still left 188
confirmed exact Web Origins without guarded terminal WEB-FINGERPRINT outcomes.
The worker then rewrote DB-provided worklist arguments, exhausted its bounded
`stage_run` retry budget, and entered automatic repair turns that could no
longer dispatch a specialist.

The same run exposed a contract split: an asset-level WEB terminal exception
could make `check_stage_asset_coverage` report `ready_to_submit=true`, while the
final exact-origin barrier correctly continued to reject every missing
`scheme://host:port` identity.

## Decision

Keep the final exact-origin barrier strict and close the upstream seams.

1. **No model-authored EAS WEB bypass.** While an authoritative exact origin is
   missing, asset-level `checked_empty`, `blocked`, or `not_applicable`
   terminal exceptions cannot close `GOLISH-EAS-WEB-FINGERPRINT` in preflight.
   Other EAS techniques retain their existing honest terminal-exception path.
   A WEB origin becomes terminal only through a guarded producer outcome (or a
   future backend-authored exact-origin recovery outcome), never through prose.
2. **Exact repair identity is backend-owned.** Repair authorization consumes the
   DB-backed `{target_id,target_url}` origin identities emitted by the worklist.
   It may accept an equivalent exact input representation, but it must not
   authorize a scheme, host, or port absent from that authoritative set. The
   existing wrapper remains the final current-owner/current-origin launch guard.
3. **Retry exhaustion is a real turn boundary.** An exhausted `stage_run` stops
   automatic same-request repair/re-entry. Only a later explicit user request
   receives a fresh bounded budget and may resume the durable worker chain.
   Repeated automatic check/submit turns must not masquerade as progress.
4. **The exact lock is live and durable.** A successful DB worklist/coverage
   refresh updates the effective repair mode before the next call in the same
   assistant tool batch and writes that refined mode to the agent-run
   checkpoint. A bounded empty page does not erase an existing lock; only an
   explicit `ready_to_submit=true` response may close it. Stage-run retries
   preserve the refined lock only while the WEB action identity is unchanged;
   a changed gap drops the stale lock and requires a fresh DB worklist read.
   A repeated `submit_stage_deliverable needs_fix` inside the same worker uses
   the same identity rule and therefore cannot erase a still-current worklist
   lock from either memory or the durable checkpoint.

## Invariants

- Do not weaken current organization, workspace, scope, target-id, or exact
  Web-Origin authorization.
- `ready_to_submit=true` must predict the same completeness result as final
  submission for EAS WEB.
- A partial WhatWeb batch keeps its successful evidence and only the remaining
  exact origins stay pending.
- Explicit user continuation may reuse durable work and stage freshness; it
  must not inherit a same-request exhausted re-entry latch.
- No schema, migration, generated IPC, or raw scanner exposure change.

## Verification

Use focused RED-to-GREEN regressions for:

- rejecting EAS WEB terminal exceptions without changing SERVICE/PORT behavior;
- authorizing only exact worklist-backed repair origins, including object and
  legacy exact-string forms, while rejecting rewritten scheme/port identities;
- stopping automatic same-request continuation after retry exhaustion while a
  new explicit user request can resume with a fresh budget; and
- applying refreshed exact targets in the same tool batch, preserving them in
  the durable checkpoint and across stage retry, and rejecting bounded-empty
  lock erasure; and
- preserving the existing exact-origin final gate and wrapper launch guards.

Focused implementation verification is complete. No live continuation was
launched from this development session because it requires restarting the
running backend and performing authorized external WhatWeb requests.
