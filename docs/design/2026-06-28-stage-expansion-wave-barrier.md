# Stage Expansion Wave Barrier

> Superseded by `docs/design/2026-06-28-eas-global-delta-expansion.md`.
> The original per-org automatic next-wave dispatch direction was replaced after
> user clarification on 2026-06-28: EAS should close seed batches across all
> organizations first, then handle newly discovered web endpoints in one global
> delta expansion pass.

> Status: Draft / approved direction by user on 2026-06-28.
> Scope: `stage_run`, EAS/enumeration coverage gates, asset landing/output-store, expansion queue, coverage UI.

## Problem

External attack surface currently uses a live asset denominator: any new `targets.scope='in'` asset that lands while a stage is running can immediately enter the coverage matrix. That is correct as DB truth, but it creates a bad operating loop:

- the worker is trying to finish the current asset set;
- scans discover more assets;
- those assets increase the denominator before the current set passes;
- the agent repeatedly submits, gets blocked, repairs a moving target, and submits again.

The UI already distinguishes `seed_assets` and `new_assets`, but the gate still uses the full live `targets.scope='in'` axis. So the interface can show "new in stage" while the deterministic gate still treats those assets as current obligations.

## Desired Behavior

Treat each active stage as a sequence of bounded asset waves:

1. `stage_run` starts wave `0` from the current in-scope assets.
2. The gate only requires wave `0` assets to reach terminal coverage.
3. Assets discovered during wave `0` are stored as DB truth and queued as pending expansion, but they do not expand wave `0`.
4. After wave `0` passes, the system performs an expansion barrier check.
5. If new eligible assets exist, promote them into wave `1` and run the same stage again for that batch.
6. Repeat until a wave passes and there are no eligible pending assets, or until bounded by max waves / max assets / human approval.

This makes progress legible:

```text
current wave: 133/133 done
newly discovered: 32 pending next wave
next action: run external_attack_surface wave 1 for 32 assets
```

## Definitions

- **Wave**: the immutable asset set a stage worker is currently responsible for.
- **Seed asset**: an asset selected into the current wave before the worker starts.
- **New asset**: an asset landed during the current wave after the wave snapshot time.
- **Expansion candidate**: a new asset that is in scope, org-linked, and eligible for a later wave.
- **Expansion barrier**: the deterministic checkpoint after a wave passes, before the operation advances to the next stage.

## Existing Hooks

- `StageAssetCoverageSummary` already exposes `seed_assets` and `new_assets`.
- `expansion_queue` already exists, but it is reviewer-only: the current migration explicitly says coverage gate does not block on it.
- `operation_state.stage_started_at` already gives a coarse freshness boundary.
- `stage_run` already has per-org pass ledgers and pass tokens.
- output-store already receives current `organization_id` through `PostShellHook` / background job completion, so active discoveries can be attributed to the current org.

## Proposed Model

### Wave Snapshot

Recommended durable model:

- add `stage_asset_waves`
  - `id`
  - `operation_id`
  - `stage_kind`
  - `organization_id`
  - `wave_index`
  - `status`
  - `started_at`
  - `completed_at`
  - `parent_wave_id`
  - `asset_hash`
- add `stage_asset_wave_items`
  - `wave_id`
  - `target_id`
  - `asset_value`
  - `asset_type`
  - `source`
  - unique `(wave_id, target_id)`

This is a schema change, so implementation requires explicit user confirmation before writing migrations.

Low-risk interim model:

- do not add schema;
- freeze the current gate axis by deriving it from `targets.created_at <= wave_started_at`;
- report `targets.created_at > wave_started_at` as `new_assets`;
- keep `expansion_queue` reviewer-only.

The interim model fixes the moving denominator for a single wave but cannot robustly resume or pass-token a wave-specific batch. The durable model is the correct endpoint.

### Asset Landing

New assets are still written immediately:

- `targets` / `target_assets` / `dns_records` / `fingerprints`;
- `technique_outcomes` for found/empty/error/blocked terminal facts;
- evidence ledger rows with real `evidence_ids`.

The change is not "hide new assets"; it is "do not add them to the current wave obligation."

### Gate Axis

For wave-aware stages, `GateContext.in_scope_assets` must be the wave item set, not the live org-wide asset set.

Applicable stages:

- `external_attack_surface`: yes, highest priority.
- `enumeration`: yes, because URL/dir/param/API discovery can also expand.
- `target_intel`: mostly already protected by `coverage_anchor_only`; keep as-is unless expansion recursion is explicitly enabled.

### Expansion Barrier

After a wave passes:

1. query new eligible assets for the same org/stage;
2. dedupe by canonical asset key and target id;
3. classify:
   - `eligible`: in-scope, org-linked, safe for this stage;
   - `skipped`: out of scope / duplicate / child already covered by host-aware rule;
   - `blocked`: needs user approval, credential, or scan authorization;
4. if eligible set is non-empty, start the next wave for the same stage before allowing graph transition;
5. if eligible set is empty, allow the stage to close normally.

### Bounds

Defaults:

- max waves per stage/org: 3;
- max promoted assets per wave: configurable cap;
- any scope expansion from out-of-scope or ambiguous ownership requires HITL approval;
- CIDR/range expansion remains subject to active-scan authorization.

## UI Contract

Coverage UI should show:

- current wave progress;
- newly discovered pending count;
- pending expansion preview;
- skipped/blocked expansion count;
- final stage completion only after the last eligible wave passes.

It should not display one flat `133/165` number without explaining that `32` are next-wave discoveries.

## Invariants

- I7: every stage fact still has evidence.
- I8: no row means not attempted; `empty` means actually checked and empty.
- Scope is not expanded silently. New assets outside confirmed scope require explicit approval before promotion.
- The gate remains deterministic. LLM self-report never decides wave completion.

## Open Decisions

1. Whether to implement the durable wave tables now, or land the interim no-schema freeze first.
2. Whether next-wave dispatch should be automatic or require a user-visible "continue next batch" confirmation for active stages.
3. Whether pass tokens should include `wave_id` and `asset_hash` immediately, or only after durable tables exist.
