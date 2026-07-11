# Stage wave origin identity and submit-preview parity

> Status: implemented and focused-verified as part of the 2026-07-10 Enumeration P0/P1 closeout.
> Scope: stage coverage read model, durable wave membership, `submit_stage_deliverable` preview.

## Problem

The Enumeration coverage snapshot expands one durable target into one or more
canonical Web Origin rows. The existing wave filter then compares each expanded
origin value with `stage_asset_wave_items.asset_value`, which still contains the
original target value. A wave member such as `app.example.com` can therefore
become `https://app.example.com:443`, be mislabeled `next_wave_pending`, and be
removed from the authoritative denominator. If every origin is removed, the
preview can vacuously pass.

The submit preview has a second parity gap. Its narrow `EvidenceLedgerQuery`
always asks for stage coverage with `current_wave=None`, while the worklist and
final per-organization gate pass the durable running wave. The three paths can
therefore grade different asset sets.

## Contract

1. Web Origin expansion must use the durable wave item's `target_id` as its
   authoritative identity. Every origin derived from one target follows that
   target into or out of the current wave together even if the target value was
   edited after the wave snapshot.
2. The identity must cover domain, IP, and URL-with-path targets, including a
   target that expands to multiple origins.
3. Explicit durable-wave membership is authoritative. A target not in the
   supplied wave is deferred; the read model must not silently re-admit it via
   `created_at`, a duplicate value, or an expanded origin string. If two targets
   materialize the same origin, the current-wave owner wins global dedupe over a
   non-wave owner.
4. Submit preview resolves the active operation and the current running wave
   from trusted server-side sources. It passes the wave `started_at` and original
   `asset_values` into the same stage coverage projection used by worklist and
   final gate.
5. Missing or mismatched operation/stage context never invents a wave. The
   existing operation-state cutoff remains the compatibility fallback only when
   the scoped repo proves there is no running wave. A present wave with zero
   items, blank values, duplicate/nil ids, mismatched id/value lengths, or a
   deleted/moved target is invalid and must error/BLOCK in worklist, submit
   preview, and final org gate; it is never equivalent to `NoWave`.
6. An Enumeration coverage snapshot read error is authoritative failure, not
   absence. Submit preview must return `needs_fix` with the snapshot error and
   must not fall back to the pre-expansion asset axis. A successful snapshot is
   accepted only after the same stage/org/session/assets-envelope validation
   used by the final per-organization gate.
7. Enumeration preview requires a bound organization, a non-empty current run
   id, a current-stage freshness cutoff, and a trusted repository. Missing
   context or `Ok(None)` is `needs_fix`; only a present snapshot containing an
   explicit `assets: []` is an authoritative zero denominator.
8. Once the snapshot has expanded and filtered the denominator to exact Web
   Origins, raw-IP/DNS-only not-applicable projection is invalid and must not be
   queried or injected by the read model, submit preview, or final org gate.

## Design

`StageAssetWaveView` carries aligned `target_ids + asset_values`. Its shared
validator distinguishes `NoWave` from a present, valid wave and rejects corrupt
membership before any fallback. The DB bridge derives both vectors from the same
ordered item rows.

`stage_asset_coverage_snapshot` accepts both vectors, validates them, proves that
every wave target id still exists in the current org/scope read, and filters by
`TargetCoverageRow.id`. Origin expansion receives the current id set; global
dedupe keeps the first stable owner unless a later owner is in the current wave
and the existing owner is not. This handles changed values and equal values on
different target ids without changing the rendered origin identity.

`EvidenceLedgerQuery::stage_asset_coverage` receives optional current-wave ids
and values and returns `Result<Option<Value>>`, preserving an actual projection
error instead of converting it to `None`. Its narrow
`stage_asset_wave_current_running` method returns the trusted
`StageAssetWaveView`; the concrete DB bridge delegates to the existing
organization/operation/stage-scoped wave repo. `SubmitStageDeliverableTool`
forwards one validated wave's `started_at + target_ids + asset_values` to the
snapshot, propagates projection errors as `needs_fix`, and reuses the final
gate's snapshot-envelope validator. Invalid present wave becomes an explicit
`needs_fix`; final org gate returns `Block`; worklist tools return an error.
The preview also fails closed when its organization, session, cutoff, repository,
or snapshot is absent. The three exact-origin consumers no longer query the
legacy raw-host not-applicable helper.

No schema or migration changes are required. Durable `target_id` already is the
DB membership key; this change carries it through the read/gate paths without
changing public UI rows.

## Verification

- Red/green unit regressions for domain, IP, URL path, multi-origin, and foreign
  target wave membership.
- Submit preview mock proving one trusted running wave's ids, values, and cutoff
  all reach the Enumeration coverage projection.
- Submit preview red/green mock proving a running-wave snapshot error is surfaced
  as `needs_fix` rather than swallowed into a raw-axis fallback.
- Focused `golish-agent-app` / `golish-agent-kit` tests, checks, clippy, fmt, and
  `git diff --check`.

Implemented evidence: the pre-fix multi-origin wave regression failed because
`app.example.com` became `http://app.example.com:80` / `https://app.example.com:443`;
the shared-origin regression also proved the old owner won before wave filtering,
and the pre-fix submit regression used operation start instead of wave start.
After the target-id/hash fix, `golish-agent-app` passed 217/217 tests,
`golish-agent-kit` passed 882/882, `golish-agent-runtime` passed 302/302, and the
focused `golish-db` wave-hash tests passed 2/2. Focused check/clippy/fmt completed
without warnings.
