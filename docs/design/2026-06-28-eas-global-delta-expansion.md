# EAS Global Delta Expansion

> Status: partial implementation started 2026-06-28.
> Supersedes `docs/design/2026-06-28-stage-expansion-wave-barrier.md`.
> Scope: `stage_run`, EAS asset batching, active-discovered web endpoints, coverage UI/read models.

## Problem

The previous durable wave design solved one real problem: discoveries during EAS must not move the current gate denominator. But its Phase 4 behavior was wrong for the product workflow: after one org passed its gate, runtime immediately promoted that org's new assets into another wave and kept running the same org.

For a real engagement with many subsidiaries, that creates poor scheduling:

- org A can recurse into wave 2 before org B-L have closed their seed assets;
- the UI looks like one company is "done then not done again";
- most active discoveries are open ports or HTTP(S) service endpoints, not brand-new root assets;
- repeating a full EAS run for every per-org wave does too much work.

## Desired Behavior

EAS should run as:

```text
seed batch for all in-scope orgs
  -> collect expansion backlog while scans run
  -> close seed batch gates for every org
  -> run one global delta expansion pass for newly discovered web endpoints / true new hosts
  -> close EAS
```

The current batch stays deterministic. Newly discovered assets are visible but do not block it. Expansion is not hidden; it is a later, explicit delta pass across the whole engagement.

## Asset Semantics

- Open ports are service facts on the scanned host/IP/domain.
- Non-web ports such as SSH, MySQL, RDP, Redis, or VPN should be stored as ports/fingerprints and not promoted into the asset denominator.
- HTTP(S) or web-like services may be promoted into URL endpoint assets, for example `https://host:8443`.
- A truly new host/IP/domain discovered from structured output may become a new target.
- URL endpoint assets should not require another PORT/SERVICE-FINGERPRINT cell; their host-level asset covers that. They need liveness/web fingerprinting and then feed Enumeration.

## Current Implementation Adjustment

`stage_asset_waves` still provides a durable immutable batch snapshot and current-batch gate axis. However, `stage_run` must not automatically call `stage_asset_wave_create_next` after a per-org PASS.

Current partial behavior:

- current batch denominator is frozen by durable wave items;
- post-batch discoveries remain visible as `new_in_stage` / `next_wave_pending`;
- per-org completion is recorded once the current batch passes;
- no immediate per-org next-wave dispatch occurs;
- after every org seed batch passes, runtime queues durable delta batches for orgs that have newly discovered targets and withholds the close pass token until a later `stage_run` processes those batches.

## Future Delta Pass

The follow-up implementation should add a global EAS expansion pass that:

1. reads all active-discovered rows for the operation/stage after the seed batch start;
2. classifies candidates as `web_endpoint`, `new_host`, `service_fact_only`, `duplicate`, `blocked`, or `out_of_scope`;
3. promotes only web-like ports/services into URL targets;
4. batches the delta across all orgs, preserving organization attribution;
5. probes only the minimal missing dimensions for the delta set;
6. enforces caps and HITL for ambiguous scope or risky expansion.

`expansion_queue` can be reused as the audit/backlog table, but it needs typed EAS leads and processed/skipped transitions before it can become the scheduling source of truth.

## Invariants

- The gate remains deterministic and evidence-backed.
- "Checked empty" still requires a real terminal probe/outcome.
- New discoveries never silently expand the current batch denominator.
- Expansion must not blindly turn every `host:port` into a target.
- The global delta pass must be bounded to avoid recursive scan explosion.
