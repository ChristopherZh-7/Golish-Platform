# EAS Global Delta Expansion Plan

## Goal

Replace per-org automatic next-wave dispatch with a seed-batch plus global delta expansion model:

```text
all org seed batches pass -> aggregate new web endpoints/new hosts -> run one bounded delta pass
```

## Phase 1: Stop Per-Org Auto-Wave Dispatch

Status: implemented 2026-06-28.

Tasks:

1. Keep durable `stage_asset_waves` for current-batch denominator freezing.
2. After a wave-aware org gate passes, mark the current wave completed.
3. Do not call `stage_asset_wave_create_next` from `stage_run_call.rs`.
4. Record `org_stage_completions` immediately after the current batch passes.
5. Update worker objective text so `next_wave_pending` means expansion backlog for a later global delta pass.
6. After all org seed batches pass, queue delta batches for every org with newly discovered targets.
7. Withhold the stage close pass token while delta batches are queued, so the main agent re-runs `stage_run` before closing EAS.

Verification:

```bash
cd backend && cargo nextest run -p golish-agent-runtime stage_asset_wave_instruction_pins_current_batch --status-level fail
cd backend && cargo check -p golish-agent-runtime
```

## Phase 2: EAS Expansion Candidate Read Model

Status: pending.

Tasks:

1. Add a read helper for post-seed active discoveries grouped by organization.
2. Classify candidates:
   - `web_endpoint`: HTTP(S) port/service, promotable to URL;
   - `new_host`: new IP/domain/URL target that needs minimal EAS delta;
   - `service_fact_only`: non-web port/service to keep on the host;
   - `duplicate`;
   - `blocked` / `out_of_scope`.
3. Surface counts in `check_stage_asset_coverage` and UI summary.
4. Keep the current batch ready state separate from expansion backlog.

Suggested verification:

```bash
cd backend && cargo nextest run -p golish-agent-app stage_asset_coverage --status-level fail
```

## Phase 3: Controlled Web Endpoint Promotion

Status: pending.

Tasks:

1. From port/service output, promote only web-like services:
   - standard web ports such as 80, 443, 8080, 8443;
   - service/banner/fingerprint hints containing HTTP(S)-like protocols.
2. Create URL targets such as `https://host:8443` with `source='active_discovered'` and org attribution.
3. Do not create targets for arbitrary non-web ports.
4. Preserve raw port/service facts on the host target for reporting and downstream vuln triage.

Suggested verification:

```bash
cd backend && cargo nextest run -p golish-pentest output_store --status-level fail
cd backend && cargo nextest run -p golish-db coverage_truth --status-level fail
```

## Phase 4: Global Delta Stage Runner

Status: pending.

Tasks:

1. Add a bounded delta runner after all org seed batches pass.
2. Consume the classified expansion backlog across all orgs.
3. Probe only the missing dimensions for delta assets:
   - URL endpoint: liveness/web fingerprint, then handoff to Enumeration;
   - new host/IP/domain: minimal EAS liveness/port/service.
4. Mark expansion leads processed/skipped/blocked.
5. Add run-tree output for seed batch and global delta pass.

Suggested verification:

```bash
cd backend && cargo nextest run -p golish-agent-runtime stage_run --status-level fail
python3 scripts/run_tree.py --workspace <ws> --full --db
```

## Rollout Notes

- Phase 1 is the safety fix: it removes the per-org recursion that confused EAS progress.
- Phases 2-4 should be built as a separate implementation slice because they change scheduling and backlog ownership.
- Full `just precommit` remains required before marking the feature passing.
