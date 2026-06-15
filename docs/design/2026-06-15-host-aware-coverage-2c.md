# 2026-06-15 · Host-aware coverage — Phase 2c (full tier)

> **This is Phase 2c (the "full" tier) of host-aware coverage.** It builds on
> `docs/design/2026-06-15-host-aware-coverage.md` (§4.1–4.4 reference + §5 "full"
> + §8 open questions) and assumes **2a (target_intel) + 2b (EAS/enumeration
> matrices) are landed** (commits `342eb54a`, `e12a7638`). 2a/2b are *drop-only*
> with *value-inferred* classification; 2c makes classification **authoritative**,
> cleans up the **truth projection**, and adds **IP-native required techniques**.
>
> Status: **DESIGN — needs user sign-off before any merge** (AGENTS.md I7/I8:
> gate-validator + recon-collector + DB change). No code written. Plan:
> `docs/superpowers/plans/2026-06-15-host-aware-coverage-2c.md`.
> Evidence below was read from source on 2026-06-15.

## 0. One sentence

2a/2b stopped a bare IP from being *falsely* asked for domain-only techniques by
*inferring* its type from the asset string and *dropping* those cells; 2c makes
the type **authoritative** (from `targets.type`), stops the truth layer from
stamping domain-only org facts onto IPs, and gives IPs their **own** required
techniques (reverse-DNS / IP-WHOIS) so an IP is *positively* covered, not merely
"excused".

## 1. What 2a/2b left open (evidence)

1. **Classification is value-inferred, not authoritative.**
   `coverage_complete` (`golish-agent-kit/src/harness/gate/rule_engine.rs:325`)
   resolves each asset's class via `AssetClass::from_value(asset)`
   (`technique_resolver.rs:36`) because `GateContext`
   (`rule_engine.rs:194`) carries only
   `in_scope_assets: Option<Vec<String>>` — **values, no type**. The hook
   (`task_orchestrator/subtask_phases/execute.rs:1960`) builds
   `GateContext { in_scope_assets, expected_techniques, evidence_facts }`; the
   authoritative `(value,type)` is fetched **separately** for the *headline*
   union (`fetch_in_scope_target_types_for_gate`, execute.rs:1226 → DISTINCT
   types → `gate_expected_techniques`, execute.rs:1714) but never reaches the
   per-asset loop. Consequence: a domain whose value looks like a CIDR (e.g.
   `10.x` style hostnames) or a typed-but-odd asset can be misclassified;
   `from_value`'s fallbacks (`""`→`Other`, unknown→`Domain`) are conservative but
   not exact.
2. **The truth layer still stamps domain-only org facts on IPs.**
   `assemble_truth_facts` (`golish-db/src/repo/coverage_truth.rs:176`) pushes the
   org-level facts (ASN/CT/WHOIS/OSINT/SUBSIDIARY) onto **every** in-scope asset
   regardless of type (lines 183–199). After 2a, the gate no longer *expects*
   `CT` on an IP, so the vacuous "CT found on 1.2.3.4" is *harmless* (never
   consulted) — but it is still **written**, which is misleading in evidence
   dumps and blocks a future "exact coverage %" readout.
3. **An IP has no positive coverage of its own.** `stage_baseline(TargetIntel)`
   (`technique_resolver.rs:64`) is the 6 domain-centric intel techniques. After
   2a an IP only needs WHOIS/ASN/OSINT (all org-level) — so an in-scope IP can
   complete `target_intel` **without any IP-specific collection at all**. There
   is no reverse-DNS / PTR or RIR/netblock-WHOIS requirement; `coverage_truth`
   has no `*_values` query for them.

## 2. Goal / scope / non-goals

- **In scope (2c):**
  - **2c-1 Authoritative type axis:** thread the real `targets.type` into
    `GateContext` so `coverage_complete` classifies per asset from DB truth
    (fallback: `from_value`, then `Other`). Reuses the existing 2a/2b
    `technique_applies` matrix — **no `expected_by_type` map needed** (simpler
    than the original §4.1 sketch).
  - **2c-2 Truth-projection refinement:** make `assemble_truth_facts` per-asset
    type-aware so domain-only org facts (CT; and the per-asset SUBDOMAIN/DNS are
    already value-gated) are **not** stamped on IP/CIDR assets.
  - **2c-3 IP-native required techniques:** add `GOLISH-INTEL-RDNS` (reverse DNS
    / PTR) + `GOLISH-INTEL-IPWHOIS` (RIR/netblock WHOIS) technique ids, their
    recon collectors + `coverage_truth` `*_values` queries, and make them
    **required** for `ip`/`cidr` in the `target_intel` matrix.
- **Out of scope / non-goals:** storage/display (Phase 0/1); the EAS/enumeration
  *flag flips* (that is 2b finalization, tracked separately); changing what 2a/2b
  already drop. **2c-3 is a distinct subsystem** (recon collectors + DB) — see §7.

## 3. Sub-phasing (each its own TDD + commit cycle)

| sub | what | risk | touches |
|---|---|---|---|
| **2c-1** | authoritative type axis in the gate | low–med (gate-core, but additive + flag-gated; fail-safe to today) | `rule_engine.rs`, `execute.rs`, `db_traits/repo.rs`, the golish-db impl of the typed read |
| **2c-2** | truth projection drops domain-only facts on IP | low (only *removes* harmless vacuous facts; gated) | `coverage_truth.rs` |
| **2c-3** | IP-native required techniques + collectors | **high / largest** (new recon capability + DB + gate baseline) | `technique_resolver.rs`, `coverage_truth.rs`, recon collectors, stage JSON, possibly migrations |

Recommended order: **2c-1 → 2c-2 → 2c-3**. 2c-1+2c-2 are self-contained (gate +
truth), reviewable, and unlock the exact-coverage readout. 2c-3 should get its
own spec after a recon-collector investigation spike (§7).

## 4. Design (the seams)

### 4.1 (2c-1) Authoritative type on the gate axis

Add one optional field to `GateContext` (additive; `None` = today's behavior):

```rust
pub struct GateContext {
    pub in_scope_assets: Option<Vec<String>>,        // unchanged (values)
    pub asset_types: Option<HashMap<String, String>>, // NEW: value -> targets.type
    pub expected_techniques: Option<Vec<String>>,     // unchanged (union headline)
    pub evidence_facts: Option<Vec<EvidenceFact>>,
}
```

In `coverage_complete`, where 2a/2b currently do
`AssetClass::from_value(asset)`, resolve the class **authoritatively first**:

```rust
let class = ctx
    .asset_types
    .as_ref()
    .and_then(|m| m.get(asset))
    .map(|ty| AssetClass::from_target_type(ty))   // existing, technique_resolver.rs:20
    .unwrap_or_else(|| AssetClass::from_value(asset)); // 2a/2b fallback, then Other
```

`AssetClass::from_target_type` already exists. **No `expected_by_type` map** — the
class plugs straight into the existing `technique_applies(spec.kind, class, tech)`
matrix (2a/2b). Fail-safe chain: authoritative type → value inference → `Other`
(full set). Flag still `host_aware_coverage` (per stage); `asset_types: None`
keeps byte-identical behavior.

The hook (`execute.rs::apply_harness_gate_hook`, ~1960) builds `asset_types` from
a typed, **org-narrowed** in-scope read (see 4.2) and injects it. A new
`fetch_in_scope_typed_assets_for_gate` mirrors `fetch_in_scope_assets_for_gate`
(execute.rs:1193) but returns `Vec<(String, String)>` (value, type).

### 4.2 (2c-1) A typed, org-narrowed in-scope read

`db_traits::repo::in_scope_targets()` (`db_traits/repo.rs:176`) today returns an
untyped `Vec<serde_json::Value>` and is **not** org-narrowed (no `org_id`). Add a
purpose-built trait method (keeps the gate pure; the query lives in golish-db):

```rust
/// In-scope (value, targets.type) pairs for an org (None = all in-scope).
/// Powers host-aware coverage's authoritative asset classification.
async fn in_scope_typed_assets(&self, org_id: Option<Uuid>)
    -> anyhow::Result<Vec<(String, String)>> { Ok(Vec::new()) } // default: empty
```

golish-db impl: `SELECT DISTINCT value, target_type::text FROM targets WHERE
scope::text='in' AND ($1 IS NULL OR organization_id=$1)` (mirrors
`coverage_truth::build_in_scope_values_sql`, coverage_truth.rs:97).

### 4.3 (2c-2) Type-aware truth projection

`assemble_truth_facts` (coverage_truth.rs:176) gains the per-asset type (passed in
alongside `in_scope_assets`, e.g. a parallel `&[String]` of types or a
`&HashMap<String,String>`). The org-level pushes become type-gated:

- `CT` (and any future domain-only org tech) is **skipped for `ip`/`cidr`**.
- `ASN`/`WHOIS`/`OSINT`/`SUBSIDIARY` stay (they genuinely apply to an IP/org).
- per-asset `SUBDOMAIN`/`DNS` are already value-set gated (no change).

This is purely *removing* a vacuous `found`; because 2a already stops *expecting*
CT on an IP, behavior is unchanged — the benefit is a clean evidence ledger and a
truthful coverage %. Gate stays a pure function; only the projected facts change.

### 4.4 (2c-3) IP-native required techniques

Add ids in `coverage_truth.rs` + baseline + matrix:

```rust
pub const TECH_RDNS: &str = "GOLISH-INTEL-RDNS";       // reverse DNS / PTR
pub const TECH_IPWHOIS: &str = "GOLISH-INTEL-IPWHOIS"; // RIR / netblock WHOIS
```

- `stage_baseline(TargetIntel)` (technique_resolver.rs:64) gains RDNS + IPWHOIS.
- `technique_applies(TargetIntel, …)`: RDNS/IPWHOIS apply to **`Ip`/`Cidr`** only
  (the mirror of SUBDOMAIN/DNS being domain-only); Domain/URL drop them.
- new `coverage_truth` `*_values` queries (`build_rdns_values_sql` from a
  PTR store / `dns_records` PTR rows; `build_ipwhois_values_sql` from an IP-WHOIS
  store) + `TruthInputs` fields + `assemble_truth_facts` per-asset pushes.
- **recon collectors** to populate them (PTR lookup; RIR WHOIS) — the largest
  piece; needs the recon-collector investigation spike (§7).

After 2c-3, an in-scope IP is *positively* required to have RDNS + IP-WHOIS +
ASN/WHOIS/OSINT, instead of passing on org-level facts alone.

## 5. Open questions (resolved defaults for the plan)

1. **`expected_by_type` map vs reuse `technique_applies`?** → **Reuse
   `technique_applies`** with an authoritative class (4.1). Less surface, one
   matrix, already unit-tested. (Supersedes design §8 Q3 default.)
2. **New typed read vs extend `in_scope_targets`?** → **New
   `in_scope_typed_assets(org_id)`** (4.2): typed + org-narrowed + minimal;
   leaves the untyped `in_scope_targets` untouched.
3. **IP-native ids** → `GOLISH-INTEL-RDNS` + `GOLISH-INTEL-IPWHOIS` (distinct
   from domain `WHOIS`). (Design §8 Q2: 2c adds them as *required*.)
4. **Backward compat** → every new `GateContext`/`TruthInputs` field defaults to
   empty/None ⇒ byte-identical when `host_aware_coverage` is off (I10).

## 6. Rollout, flag, parity (risk I7/I8)

- Same per-stage `host_aware_coverage` flag. 2c-1/2c-2 ride the **already-on**
  `target_intel` flag; 2c-3 changes the baseline so it needs a fresh parity pass.
- **Parity test discipline (mandatory before any flag-affecting merge):** capture
  gate PASS/BLOCK on a known mixed domain+IP `--stage-run`, flag off vs on; the
  only allowed delta per sub-phase:
  - 2c-1: a *misclassified* asset's decision corrects; **no** correctly-typed
    asset changes.
  - 2c-2: **zero** gate-decision delta (only ledger facts shrink).
  - 2c-3: IPs gain RDNS/IPWHOIS *required* cells (may BLOCK until collectors run)
    — so 2c-3 ships its collectors + flag together, behind its own sign-off.
- **Unit coverage:** typed-class resolution (authoritative beats value; value
  fallback; Other fail-safe); `assemble_truth_facts` skips CT on IP; RDNS/IPWHOIS
  matrix + truth.
- **Explicit user sign-off before merge** (harness-core; AGENTS.md §2.5/§2.7).

## 7. Touch points (files)

- `golish-agent-kit/src/harness/gate/rule_engine.rs` — `GateContext.asset_types`;
  `coverage_complete` authoritative class resolution.
- `golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs` —
  `fetch_in_scope_typed_assets_for_gate`; build `asset_types`; inject (≈1960).
- `golish-agent-kit/src/db_traits/repo.rs` — `in_scope_typed_assets(org_id)`
  trait method (default empty).
- golish-db `RepoProvider` impl (wherever the `db_traits` impls live) — the typed
  in-scope SELECT.
- `golish-db/src/repo/coverage_truth.rs` — (2c-2) type-aware `assemble_truth_facts`;
  (2c-3) RDNS/IPWHOIS ids + `*_values` queries + `TruthInputs`.
- `golish-agent-kit/src/harness/technique_resolver.rs` — (2c-3) baseline +
  matrix arms for RDNS/IPWHOIS.
- **recon collectors** (2c-3) — PTR lookup + RIR WHOIS. **NOT yet located**; needs
  an investigation spike (grep the recon collector framework, e.g. how
  `dns_records` / `whois` get populated) before a real task breakdown. The 2c-3
  plan must start with that spike, then split into its own spec if large.
- `resources/harness/stages/target_intel.json` — (2c-3) any new baseline keys
  (flag already on).

## 8. Risks / non-goals

- Risk: authoritative type *narrowing* a class that value-inference kept broad
  could newly relax a cell — mitigated by parity test + fail-safe fallback chain
  (authoritative → value → Other-full).
- Risk: 2c-3 collectors are real network actions (PTR/WHOIS) → respect scope +
  rate limits; design the collectors under `docs/design/` per AGENTS.md §2.5 if
  they widen the active surface.
- Non-goal: 2c does **not** finalize 2b's flag flips (separate), nor add active
  scanning beyond passive PTR/WHOIS.

## 9. Self-check

- Spec coverage: §1.1→2c-1 (4.1/4.2), §1.2→2c-2 (4.3), §1.3→2c-3 (4.4).
- Types consistent: `asset_types: Option<HashMap<String,String>>`,
  `in_scope_typed_assets(org_id) -> Vec<(String,String)>`,
  `AssetClass::from_target_type`, `technique_applies(stage,class,tech)`,
  `TECH_RDNS`/`TECH_IPWHOIS` used identically across §4 and the plan.
- Reuse over new: explicitly drops design §4.1's `expected_by_type` map in favor
  of the existing matrix (§5 Q1).
