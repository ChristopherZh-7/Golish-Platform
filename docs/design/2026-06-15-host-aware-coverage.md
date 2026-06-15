# 2026-06-15 · Host-aware coverage (per-asset-type technique matrix)

> **This is Phase 2 (Option C) of the IP-centric asset model**
> (`docs/design/2026-06-15-ip-centric-asset-model.md` §2 Option C / §4 Phase 2).
> Phase 0 (persist `real_ip`) and Phase 1 (IP-centric tree) are done; they only
> touched **storage + display**. This phase touches the **harness gate truth**
> (coverage), so it is a separate, higher-risk change.
>
> Status: **DESIGN — approved scope (2a), needs sign-off before merge** (AGENTS.md
> I7/I8: gate-validator change). No code written yet. Implementation plan:
> `docs/superpowers/plans/2026-06-15-host-aware-coverage.md` (Phase 2a, per the
> §8 defaults: 2a-only / drop-only / URL inherits host / classify-from-value).
> Evidence below was read from source on 2026-06-15.

## 0. One sentence

Today the coverage gate demands **one** expected-technique list from **every**
in-scope asset regardless of its type; this phase makes the demanded techniques
a function of **asset type** (domain vs IP vs URL), per stage — so a bare IP is
no longer asked for "subdomains" and a domain is no longer asked for a "port
scan".

## 1. Problem (evidence)

Coverage is computed per stage by `coverage_complete`
(`golish-agent-kit/src/harness/gate/rule_engine.rs`). Its core is a **type-blind
double loop**:

```
for asset in assets {        // ctx.in_scope_assets — VALUES only (Vec<String>)
    for tech in techniques { // ctx.expected_techniques — ONE list for the scope
        // gap unless a terminal cell/fact exists for (asset, tech)
    }
}
```

Two structural facts make it type-blind:

1. **The asset axis carries no type.** `GateContext.in_scope_assets:
   Option<Vec<String>>` (rule_engine.rs:195) is a list of target **values**
   only. It is filled by `fetch_in_scope_assets_for_gate` →
   `repo.in_scope_assets(org_id)` (a `SELECT DISTINCT value ...`,
   `golish-db/src/repo/targets.rs::build_list_in_scope_values_legacy_sql`).
2. **Expected techniques are scope-level, not per-asset.**
   `GateContext.expected_techniques: Option<Vec<String>>` is **one** list,
   produced by `gate_expected_techniques(stage, target_types)`
   (`task_orchestrator/subtask_phases/execute.rs:1714`) →
   `DefaultSprintContractGenerator::expected_techniques_for`
   (`harness/sprint_contract.rs:121`) → `resolve_expected_techniques`
   (`harness/technique_resolver.rs:66`). That resolver takes the **union** of the
   scope's asset types (`in_scope_target_types`, DISTINCT types) and emits a
   single list. Its only type pruning is one coarse scope-level rule: drop
   `GOLISH-ENUM-PARAM` when the whole scope has no web asset.

So for `target_intel` (baseline 6: DNS, SUBDOMAIN, CT, WHOIS, ASN, OSINT —
`technique_resolver.rs::stage_baseline`) every in-scope asset is required to have
all 6. Concrete consequence for an in-scope **IP** `1.2.3.4`:

- `GOLISH-INTEL-SUBDOMAIN` — per-asset truth is `subdomain_values.contains(asset)`
  (`coverage_truth.rs::assemble_truth_facts`); an IP is never a subdomain key →
  **gap forever**.
- `GOLISH-INTEL-DNS` — per-asset truth is `dns_values.contains(asset)`
  (forward A keyed by the **domain**); an IP has no forward record → **gap
  forever**.
- `GOLISH-INTEL-CT` / `ASN` / `WHOIS` / `OSINT` — these are **org-level** facts
  (`has_ct`/`has_asn`/...), stamped on **every** asset including the IP → the IP
  "passes" CT **vacuously** (a cert-transparency cell on a bare IP is
  meaningless).

Net: an in-scope IP can **never** legitimately complete `target_intel` (two
unsatisfiable cells), so the stage either BLOCKs forever or the agent fakes the
cells to get through. That is exactly the "coverage logic for intel is imprecise"
problem.

## 2. Goal / scope

- **In scope:** make the *expected*-technique set a function of `(stage, asset
  type)`, and thread per-asset type into the gate so `coverage_complete` selects
  each asset's techniques. Start with `target_intel` (the passive stage), then
  apply the same shape to `external_attack_surface` and `enumeration`.
- **Out of scope:** changing what recon tools run, adding new collectors (e.g. a
  reverse-DNS collector) — those are optional follow-ups (§5 "full" tier), not
  required to fix the false-incomplete problem.
- **Non-goal:** changing storage/display (that was Phase 0/1).

## 3. The technique × asset-type matrix

Per stage, which of the stage's baseline techniques each asset type must satisfy.
`✓` = required, `—` = not applicable (excluded from that asset's gap loop),
`(opt)` = appropriate but needs a new collector (full tier only).

### 3.1 `target_intel` (passive · zero-touch)

| technique | domain / subdomain | ip | cidr | url |
|---|---|---|---|---|
| `GOLISH-INTEL-DNS` (forward A/AAAA) | ✓ | — | — | ✓(host) |
| `GOLISH-INTEL-SUBDOMAIN` | ✓ | — | — | — |
| `GOLISH-INTEL-CT` (cert transparency) | ✓ | — | — | ✓(host) |
| `GOLISH-INTEL-WHOIS` | ✓ | ✓ (IP/netblock whois) | ✓ | ✓(host) |
| `GOLISH-INTEL-ASN` | ✓ | ✓ | ✓ | ✓(host) |
| `GOLISH-INTEL-OSINT` | ✓ | ✓ (org-level) | ✓ | ✓ |
| `(opt) reverse DNS / PTR` | — | (opt) | — | — |

The key change: **IP/CIDR drop DNS-forward / SUBDOMAIN / CT** (the domain-only
items). WHOIS/ASN/OSINT remain (genuinely apply to an IP/org).

### 3.2 `external_attack_surface` (active · touches host)

Baseline LIVENESS / PORT / SERVICE-FINGERPRINT — these are **host-level**:

| technique | domain | ip / host | url |
|---|---|---|---|
| `GOLISH-EAS-LIVENESS` | ✓ (resolved host) | ✓ | ✓(host) |
| `GOLISH-EAS-PORT` | ✓ (via resolved IP) | ✓ | — |
| `GOLISH-EAS-SERVICE-FINGERPRINT` | ✓ | ✓ | — |

### 3.3 `enumeration` (active · content)

Baseline DIR / PARAM / JSAPI — **URL/web-level** (the existing coarse rule
already drops PARAM for no-web scopes; this makes it per-asset):

| technique | domain | ip | url |
|---|---|---|---|
| `GOLISH-ENUM-DIR` | ✓ | — (unless web) | ✓ |
| `GOLISH-ENUM-PARAM` | ✓ | — | ✓ |
| `GOLISH-ENUM-JSAPI` | ✓ | — | ✓ |

## 4. Design (the seam change)

### 4.0 Chosen implementation for 2a (refined — simpler than §4.1–4.3)

The implementation plan (`docs/superpowers/plans/2026-06-15-host-aware-coverage.md`)
realizes 2a **without** any `GateContext`/hook/DB change, because
`coverage_complete(d, spec, ctx, …)` **already** receives both `&StageSpec` (with
`spec.kind`) and `ctx.in_scope_assets` (the values). So 2a is just:

1. `StageSpec` gains a `host_aware_coverage: bool` flag (default false; mirrors the
   existing `facts_from_db_truth` flag) — set true only in `target_intel.json`.
2. `technique_resolver` gains `AssetClass::from_value(&str)` (infer type from the
   asset *value*: parses as `IpAddr` → `Ip`; `http(s)://` → `Url`; `addr/prefix`
   → `Cidr`; else `Domain`) and `technique_applies(stage, class, tech) -> bool`
   (the §3 matrix predicate).
3. `coverage_complete`: when `spec.host_aware_coverage`, the inner technique loop
   **filters `techniques` per asset** via `technique_applies(spec.kind, class,
   tech)`; flag off ⇒ byte-identical to today.

§4.1–4.3 below describe the heavier "typed axis in GateContext" variant — keep it
as the reference for 2c (when authoritative `targets.type` and IP-native
techniques are needed); 2a uses the simpler value-classification above.

### 4.1 (reference, for 2c) Carry asset type on the axis

Two coordinated changes; the gate stays a **pure, DB-free function**.

### 4.1 Carry asset type on the axis

The `(value, type)` data already exists: `db_traits::repo::in_scope_targets()`
returns rows with `value`/`type`, and `targets` rows are typed. Add a typed
in-scope read (or reuse `in_scope_targets`) so the hook can build an
`asset → AssetClass` map. `AssetClass::from_target_type`
(`technique_resolver.rs:20`) already maps the `targets.type` strings.

Extend `GateContext` so `coverage_complete` knows each asset's type. Recommended
concrete shape (least churn, additive, default-None = today's behavior):

```rust
pub struct GateContext {
    pub in_scope_assets: Option<Vec<String>>,        // unchanged (values)
    pub asset_types: Option<HashMap<String, String>>, // NEW: value -> targets.type
    pub expected_techniques: Option<Vec<String>>,     // unchanged (stage baseline)
    pub expected_by_type: Option<...>,                // NEW: per-AssetClass matrix
    pub evidence_facts: Option<Vec<EvidenceFact>>,
}
```

### 4.2 Per-asset technique selection in `coverage_complete`

Replace `for asset { for tech in techniques }` with: for each asset, resolve its
`AssetClass` (from `asset_types`, default `Other` = "keep full list" so unknowns
never wrongly relax), then iterate **that type's** techniques (from
`expected_by_type`, falling back to the flat `expected_techniques` when the map
is absent). A `—` cell never enters the gap loop. **Fail-safe default:** missing
type info ⇒ full list (never silently relax the gate).

### 4.3 Expected-technique generator becomes per-type

`technique_resolver::resolve_expected_techniques` currently takes a `&[AssetClass]`
(the scope union). Add a per-type variant `techniques_for(stage, AssetClass) ->
Vec<String>` implementing §3's matrix; keep the scope-union function for the
"declared techniques" headline. The gate hook builds `expected_by_type` for the
asset classes present in scope.

### 4.4 (Full tier, optional) refine the truth projection

`assemble_truth_facts` stamps org-level techniques (CT/ASN/WHOIS/OSINT) on every
asset. Once §4.2 stops *expecting* CT on an IP, the vacuous CT-on-IP `found` is
harmless (never consulted). The full tier would additionally (a) stop stamping
domain-only org techniques on IPs and (b) add IP-native collectors
(reverse-DNS/PTR, IP-WHOIS) + their `*_values` truth queries. **Not required**
for the false-incomplete fix.

## 5. Phasing within Phase 2

- **2a (recommended first, minimal, lower risk):** §4.1–4.3 for `target_intel`
  only, behind a per-stage flag (see §6). Fixes the IP false-incomplete with no
  new collectors. The truth side is untouched.
- **2b:** apply §4.2–4.3 matrix to `external_attack_surface` + `enumeration`
  (mostly formalizes the existing coarse PARAM rule per-asset).
- **2c (full):** §4.4 truth-projection refinement + optional IP-native
  techniques. Largest, needs new DB queries / recon wiring.

## 6. Rollout, flag, and the mandatory parity test (risk I7/I8)

This is the gate validator. Miscalibration = wrongly PASS (security: an
incomplete recon slips the gate) or wrongly BLOCK (workflow deadlock). So:

- **Gradual flag**, mirroring prior coverage phases (a per-stage switch in
  `resources/harness/stages/target_intel.json`, default **off**); `GateContext`
  fields default `None` = byte-identical to today.
- **PASS/BLOCK parity test before flipping on:** pick a known headless
  `--stage-run` (e.g. one with a mixed domain+IP scope), capture the gate
  decisions with the flag off, then on, and diff. The only allowed delta is:
  IP/CIDR assets that were BLOCKed on `SUBDOMAIN`/`DNS`/`CT` now PASS those cells;
  **no** domain asset's decision may change. Add this as an explicit test.
- **Unit coverage:** `techniques_for(stage, class)` for every (stage × class),
  plus `coverage_complete` selecting per-asset (a domain still needs 6, an IP
  needs the reduced set, an `Other`/unknown keeps the full set).
- **Explicit user sign-off** required before merge (harness-core change).

## 7. Touch points (files)

- `golish-agent-kit/src/harness/gate/rule_engine.rs` — `GateContext` (+asset
  types / per-type matrix); `coverage_complete` per-asset technique selection.
- `golish-agent-kit/src/harness/technique_resolver.rs` — add `techniques_for(
  stage, AssetClass)` (the §3 matrix); keep `resolve_expected_techniques`.
- `golish-agent-kit/src/harness/sprint_contract.rs` — expose the per-type variant
  alongside `expected_techniques_for`.
- `golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs` — the gate
  hook: fetch per-asset `(value, type)` (via `in_scope_targets` or a new typed
  read), build `asset_types` + `expected_by_type`, inject into `GateContext`
  (`fetch_in_scope_assets_for_gate` / `fetch_in_scope_target_types_for_gate` /
  `apply_harness_gate_hook` / `gate_expected_techniques`).
- `golish-agent-kit/src/db_traits/repo.rs` — `in_scope_targets` already returns
  `value`/`type`; confirm/extend so the hook gets typed rows org-narrowed.
- (full tier) `golish-db/src/repo/coverage_truth.rs::assemble_truth_facts` +
  new `*_values` queries; recon collectors for PTR / IP-WHOIS.
- `resources/harness/stages/{target_intel,external_attack_surface,enumeration}.json`
  — per-type expectations + the gradual flag.

## 8. Open questions (confirm before the plan)

1. **Scope of first cut:** ship **2a (target_intel only)** first? Default: **yes**.
2. **IP-native techniques:** add reverse-DNS/PTR + IP-WHOIS as *required* for IPs,
   or just **drop** the domain-only ones for IPs (no new collectors)? Default:
   **drop only** (2a); add IP-native later (2c).
3. **`expected_by_type` representation:** `HashMap<AssetClass, Vec<String>>` in
   `GateContext` vs. resolving per-asset on the fly inside `coverage_complete`.
   Default: **map in GateContext** (keeps the gate pure, easy to unit-test).
4. **URL handling at passive stage:** treat a URL's intel as its host's
   (inherit), or `—`? Default: **inherit host (domain) techniques**.

## 9. Risks / non-goals

- Non-goal: storage/display (Phase 0/1), new recon tooling (unless 2c).
- Risk: relaxing the wrong cell weakens a real gate. Mitigated by fail-safe
  default (unknown type ⇒ full list), the parity test, and default-off flag.
- Risk: `AssetClass::Other` over-relaxing — it deliberately keeps the **full**
  list, so an unclassified asset is never under-checked.
