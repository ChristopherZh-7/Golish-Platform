# EAS Domain Port/Service Delegation To In-Scope IP

> Date: 2026-06-30
> Status: implementing
> Related: `docs/design/2026-06-15-host-aware-coverage.md`,
> `docs/design/2026-06-24-intel-to-eas-handoff.md`,
> `resources/harness/stages/external_attack_surface/spec.json`,
> `resources/harness/stages/external_attack_surface/methodology.md`
> Invariants touched: I7 (evidence-backed stage delivery), I8 (checked_empty != unchecked)

## 1. Problem

During a `external_attack_surface` (EAS) run on 默安科技, the operator wanted the
stage to port-scan **only IP targets**, not the domains that resolve to those
same IPs (a domain and its A-record IP are the same host; scanning both wastes
work and double-counts the surface).

A seed-collapse rule already landed in
`golish-app-core::domain::targets::collapse_attack_surface_seed_aliases`: when a
domain's `real_ip` equals an existing in-scope IP target, the domain alias is
dropped from the **attack-surface seed list** returned by
`list_attack_surface_seeds`.

But the run still BLOCKED at the EAS gate and the prober was forced to port-scan
domains (`naabu -host route.moresec.cn`, `nmap -sV -iL` over domain lists). Root
cause: the **gate coverage denominator is independent of the seed list**.

- `coverage_complete` (EAS) holds every in-scope asset to its applicable EAS
  techniques via `technique_resolver::technique_applies` /
  `technique_applies_to_value`.
- The EAS host-aware rule only drops `GOLISH-EAS-PORT` /
  `GOLISH-EAS-SERVICE-FINGERPRINT` for a bare `Url` asset. `Domain` / `Ip` /
  `Cidr` keep all three.
- So every in-scope **domain** is still required to have terminal PORT +
  SERVICE-FINGERPRINT cells. With no port scan on the domain and no
  `not_applicable` note, those cells are non-terminal → `coverage_complete`
  BLOCK.

The seed collapse changed *what the worker is told to scan*; it did not change
*what the gate counts*. The two must agree.

## 2. Decision (variant A — align the authoritative gate with the precheck)

Discovery during implementation: the read-only precheck
(`stage_coverage.rs::eas_alias_coverage_cells`) ALREADY delegates such aliases —
when an asset's resolved IP is an in-scope IP target it marks ALL its EAS
techniques (LIVENESS/PORT/SERVICE-FINGERPRINT) `not_applicable`. But the
authoritative gate (`org_gate` → `rule_engine` coverage_complete) had no such
logic, so the two coverage computations disagreed. That mismatch — not a missing
seed collapse — is what forced the prober to port-scan domains and what blocked
the run.

Decision: an in-scope asset (domain/url) whose resolved IP is already an in-scope
IP target has ALL its EAS techniques delegated to that IP target. This aligns the
authoritative gate with the precheck (variant A), the smallest change that makes
the two consistent. Liveness is delegated too (the original "keep LIVENESS"
variant B was dropped to match the precheck the user approved).

Why precise (only when a concrete IP target exists): an asset whose resolved IP
is NOT an in-scope IP target has no IP asset to carry its coverage, so it is NOT
delegated and still requires its own EAS techniques (no silent unscanned host).

Implementation (no `GateContext` field): rather than add a struct field (which
would touch ~27 manual `GateContext { .. }` test constructions), the authoritative
gate and the submit preview REMOVE these alias assets from the in-scope asset
axis (`in_scope_assets` / `typed_assets`) before building the `GateContext`.
Removed-from-denominator == not held to any EAS technique == the precheck's
`not_applicable`. This never adds a requirement and never fabricates a found cell
(I7/I8 preserved).

## 3. Data Flow

```
targets (value, type, real_ip)
  -> eas_port_delegated_domain_values(targets)   [golish-app-core, pure]
       = non-IP assets whose resolved IP ∈ {in-scope IP target IPs}
  -> repo.eas_port_delegated_assets(org_id)       [golish-agent-app, DB read]
  -> GateContextBuilder.eas_port_delegated(set)    [golish-agent-kit, assembly]
  -> coverage_complete asset_techniques filter:    [golish-agent-kit, pure gate]
       EAS + asset ∈ set  ⇒ drop PORT / SERVICE-FINGERPRINT
```

The same set is injected at all three gate entry points so the read-only
precheck and the authoritative gate agree:

- `org_gate::evaluate_org_stage_gate` (per-org fan-out gate)
- `harness_submit_tool` (submit-time preflight)
- `stage_coverage` / `check_stage_asset_coverage` (read-only coverage precheck)

## 4. Files

| File | Change |
|---|---|
| `golish-app-core/src/domain/targets.rs` | pure `eas_port_delegated_domain_values` + unit tests |
| `golish-agent-kit/src/db_traits/repo.rs` | `eas_port_delegated_assets(org_id)` trait method (default empty) |
| `golish-agent-app/src/ai/db_bridge/mod.rs` + `recon.rs` | trait override + impl: load in-scope targets → call pure fn |
| `golish-agent-kit/src/harness/org_gate.rs` | EAS: `retain` removes alias assets from `in_scope_assets`/`typed_assets` (authoritative gate) |
| `golish-agent-app/src/ai/harness_submit_tool.rs` | `EvidenceLedgerQuery` seam + EAS `retain` (submit preflight) |
| `golish-agent-app/src/ai/db_bridge/evidence.rs` | seam impl delegating to `eas_port_delegated_assets_impl` |
| `golish-agent-app/src/ai/commands/stage_coverage.rs` | UNCHANGED — already has `eas_alias_coverage_cells` (the precheck side) |
| `resources/harness/stages/external_attack_surface/methodology.md` | domain→IP port/service delegation wording |

## 5. Verification

```bash
cd backend && cargo nextest run -p golish-app-core -p golish-agent-kit --status-level fail
cd backend && cargo clippy -p golish-app-core -p golish-agent-kit --all-targets
```

Key tests:
- `eas_port_delegated_domain_values` collapses domain alias of an in-scope IP,
  keeps a domain with no in-scope IP, returns empty when no IP targets.
- `coverage_complete` EAS: a domain in the delegated set passes with only
  LIVENESS terminal (no PORT/SERVICE); a domain NOT in the set still BLOCKs
  without PORT/SERVICE.

## 6. Out Of Scope

- The first "misleading BLOCK" (gate treating inherited target_intel passive
  evidence as EAS scan results) is a separate fix, tracked but not done here.
- No DB schema change. No new migration. The new repo method is a read.
