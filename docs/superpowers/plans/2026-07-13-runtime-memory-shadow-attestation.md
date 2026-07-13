# Runtime Memory retained shadow rollout implementation plan

**Goal:** keep migration cutover at rank 1 and make ranks 2/3 depend on a
database-authoritative, retained whole-record cohort.

## Task 1: freeze the unsafe behavior with RED tests

- Add an independent embedded-Postgres integration test file.
- Prove fresh migrations leave runtime memory at
  `dual_write_legacy_read/rank=1`.
- Prove raw adjacent rollout UPDATE and the public CAS both fail without an
  admitted, matching cohort.
- Prove a missing sample, retained mismatch, tampered hash, missing legacy
  record, identity drift and invalid runtime/attack pair each block promotion.
- Prove concurrent admission is either included before the cutoff or remains an
  older frozen-contract operation after promotion.

Run `just space-guard` before every Cargo command and record RED output before
production changes.

## Task 2: add the additive schema authority

- Add the next ordered migration after the attack rollout migration.
- Create immutable admission, shadow-sample and promotion-receipt tables.
- Backfill existing dual WorkerRuns in stable identity order.
- Add the raw-insert admission trigger and exact owner/contract constraints.
- Add canonical SQL whole-record rehydration and sample validation functions.
- Replace/extend the rollout transition trigger so rank 1/2 promotion recomputes
  readiness from relational truth and writes its own receipt.
- Keep the original rank-0 to rank-1 migration as sampling enablement only.

## Task 3: record complete samples on every dual mutation

- Add a dedicated `runtime_memory_shadow` repository module.
- Persist the sample after the legacy mirror write and before commit.
- Route seed/claim, checkpoint, heartbeat, tool lifecycle, retry/terminal, final
  seal, continuation, reaper and reset mirrors through the same helper.
- Load the just-persisted legacy JSON path and V2 row; do not compare two
  caller-built copies.
- Keep V2-only paths free of legacy writes and shadow samples.
- For V2-only developer reset, ignore the caller checkpoint blob, preserve only
  already-persisted non-checkpoint siblings, strip every legacy runtime
  namespace and write only a server-authored reset marker.

## Task 4: expose typed post-commit reconciliation

- Replace unrestricted application promotion with `reconcile` returning
  `promoted`, `unchanged_not_ready`, or `already_v2_only`.
- Invoke it only in a separate transaction after durable mirror/final-seal
  commits; optionally reconcile before a new operation freezes defaults.
- Propagate an explicit sample-written count through the startup reaper so it
  reconciles only after commit; response-loss replay with zero new samples is
  an idempotent no-op.
- At caller-owned final-seal boundaries, reconcile runtime first and Candidate
  attack rollout second because attack may depend on the runtime default.
- Preserve final-seal success if reconciliation is not ready or fails.
- Recheck runtime/attack compatibility while both deployment rows are locked in
  the documented order.

## Task 5: GREEN and regression closure

- Run focused schema, repository and app bridge tests.
- Run the complete runtime-memory migration/transaction suites and the attack
  rollout compatibility suite.
- Run scoped Clippy with `-D warnings`, then the feature verification commands
  and `just precommit`.
- Update `golish-db` module cards/index, `agent-progress.md` and
  `feature_list.json` with fresh command evidence only.
