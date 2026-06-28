# Stage Expansion Wave Barrier Plan

> Superseded by `docs/superpowers/plans/2026-06-28-eas-global-delta-expansion.md`.
> Phase 4's per-org automatic next-wave dispatch is no longer the desired EAS
> control flow. Newly discovered web endpoints should be aggregated into a
> global delta expansion pass after all org seed batches close.

## Goal

Make newly discovered assets batch-oriented: finish the current stage wave first, then check and promote a new batch, then run `stage_run` again for that batch. This prevents live asset discovery from moving the gate denominator mid-run.

## Phase 0: Current-State Confirmation

- Confirm `StageAssetCoverageSummary.seed_assets/new_assets` already separates display counts.
- Confirm `expansion_queue` is reviewer-only and not gate-enforced.
- Confirm EAS gate currently injects live `targets.scope='in'` as `GateContext.in_scope_assets`.
- Confirm output-store/background completion keeps writing real DB truth and `technique_outcomes`.

Verification:

```bash
rg -n "new_assets|seed_assets" backend/crates/golish-agent-app/src/ai/commands/stage_coverage.rs
rg -n "expansion_queue|coverage gate does NOT block" backend/crates/golish-db
rg -n "fetch_in_scope_assets_for_gate|in_scope_assets" backend/crates/golish-agent-kit/src backend/crates/golish-agent-app/src
```

## Phase 1: No-Schema Current-Wave Freeze

Status: implemented 2026-06-28 without schema changes.

Purpose: stop the moving denominator without a DB migration.

Tasks:

1. Add an explicit stage spec flag, e.g. `asset_wave_barrier: "stage_started_at"`, default off.
2. Enable it for `external_attack_surface` first.
3. Add a repo method that returns in-scope assets created at or before a cutoff timestamp.
4. In stage-close gate context assembly, when the flag is enabled and `operation_state.stage_started_at` exists, inject only cutoff assets into `GateContext.in_scope_assets`.
5. Keep DB truth facts filtered to the same injected axis.
6. Keep `stage_asset_coverage_snapshot` showing all assets, but label post-cutoff rows as next-wave pending.
7. Add tests that an asset created after the cutoff does not block current-wave EAS coverage.

Suggested verification:

```bash
cd backend && cargo nextest run -p golish-agent-kit wave coverage_complete --status-level fail
cd backend && cargo nextest run -p golish-agent-app stage_asset_coverage --status-level fail
cd backend && cargo check -p golish-agent-kit -p golish-agent-app
```

Limitations:

- This phase prevents denominator drift for the active wave.
- It does not provide durable `wave_id`, pass-token hashing, or automatic next-wave dispatch.

## Phase 2: Expansion Barrier Read Model

Status: partially implemented 2026-06-28 as the no-schema read model. The snapshot/UI now expose post-cutoff assets as `new_in_stage` / `next_wave_pending`, and `check_stage_asset_coverage` does not treat those cells as current-wave gaps. Explicit `eligible/skipped/blocked` expansion classification remains for the durable wave phase.

Purpose: make the next batch visible and actionable.

Tasks:

1. Add a read helper that returns post-cutoff eligible assets by org and stage.
2. Classify each new asset as `eligible`, `skipped`, or `blocked`.
3. Extend `check_stage_asset_coverage` with:
   - `current_wave_ready`
   - `next_wave_assets`
   - `next_wave_count`
   - `next_action`
4. Update the UI to render `current wave` and `pending next wave` separately.

Suggested verification:

```bash
cd backend && cargo nextest run -p golish-agent-app stage_asset_coverage --status-level fail
pnpm exec vitest run frontend/components/Engagement/StageAssetCoveragePanel.test.tsx
```

## Phase 3: Durable Wave Tables

Status: implemented 2026-06-28 after user approval for the additive DB migration. Pass-token hashing is still the existing org completion token; wave id/hash is recorded in `stage_asset_waves` and surfaced to the worker objective, but not yet folded into closeout token recomputation.

Purpose: support robust resume, pass-token integrity, and running only the promoted batch.

Requires explicit approval because it changes schema/migrations.

Tasks:

1. Add `stage_asset_waves` and `stage_asset_wave_items` migrations.
2. Add `golish-db` repo helpers:
   - create wave
   - list wave items
   - complete wave
   - promote eligible next-wave assets
3. Create wave `0` when `stage_run` starts a wave-aware stage/org.
4. Use wave items as the gate axis.
5. Persist wave id in `operation_state.state_blob.stage_run_workers`.
6. Include `wave_id` and `asset_hash` in the stage-run pass token.
7. On resume, recover the active wave instead of recomputing from live `targets`.

Suggested verification:

```bash
cd backend && cargo nextest run -p golish-db stage_asset_wave --status-level fail
cd backend && cargo nextest run -p golish-agent-runtime stage_run wave --status-level fail
cd backend && cargo nextest run -p golish-agent-kit pass_token wave --status-level fail
```

## Phase 4: Automatic Next-Wave Dispatch

Status: implemented 2026-06-28 for `stage_run_call.rs`: after a wave-aware org passes its per-org gate, runtime marks the wave completed, promotes unassigned in-scope targets into the next durable wave, and continues the same org until no new wave exists or the automatic cap is reached.

Purpose: run the newly discovered batch without relying on the model noticing it.

Tasks:

1. After a per-org wave passes, call the expansion barrier.
2. If eligible next-wave assets exist, start the next wave for the same org/stage.
3. Use bounded defaults:
   - max 3 waves per org/stage;
   - configurable max assets per wave;
   - HITL for ambiguous scope expansion or high-risk active scanning.
4. Only mark the org stage completion when the latest wave passes and no eligible next-wave assets remain.
5. Update `run_tree.py` to print wave summaries.

Suggested verification:

```bash
cd backend && cargo nextest run -p golish-agent-runtime stage_run wave --status-level fail
python3 scripts/run_tree.py --workspace <ws> --full --db
```

## Phase 5: Final Gates

Run scoped checks first, then full checks once the local pnpm approval gate is resolved.

```bash
cd backend && cargo check -p golish-db -p golish-agent-kit -p golish-agent-app -p golish-agent-runtime
cd backend && cargo clippy -p golish-db -p golish-agent-kit -p golish-agent-app -p golish-agent-runtime --all-targets -- -D warnings
just precommit
```

## Rollout Notes

- Phase 1 can land without schema changes and should be the first safety cut.
- Phase 3 is the durable model and needs user approval before migration work.
- Do not change `target_intel` first; its `coverage_anchor_only` already solves the most obvious passive subdomain treadmill.
