# Host-aware coverage Phase 2c — implementation plan

> **For the AI worker:** execute task-by-task with `.cursor/skills/executing-plans`
> (TDD, frequent commits). Spec: `docs/design/2026-06-15-host-aware-coverage-2c.md`.
> Assumes 2a (`342eb54a`) + 2b matrix (`e12a7638`) are landed. **Harness-core
> change → explicit user sign-off required before merging any flag-affecting task.**

**Goal:** Make host-aware coverage classify each in-scope asset from its
*authoritative* `targets.type` (2c-1), stop the truth layer from stamping
domain-only org facts on IPs (2c-2), and (separately) add IP-native required
techniques (2c-3).
**Architecture:** Additive `GateContext.asset_types` (value→type) injected by the
gate hook from a new typed, org-narrowed in-scope read; `coverage_complete`
resolves class authoritatively (fallback: `from_value` → `Other`) and reuses the
existing `technique_applies` matrix — **no `expected_by_type` map**. Truth
projection becomes type-aware. All gated behind the existing per-stage
`host_aware_coverage` flag; every new field defaults to empty ⇒ byte-identical
when off.
**Stack:** Rust (`golish-agent-kit` harness + db_traits, `golish-db`
coverage_truth + repo impl), JSON stage spec.

---

## Decisions locked (design §5)

- Reuse `technique_applies(spec.kind, class, tech)`; **no** `expected_by_type`.
- New `in_scope_typed_assets(org_id) -> Vec<(value, type)>` (don't touch the
  untyped `in_scope_targets`).
- Fail-safe class chain: authoritative `from_target_type` → `from_value` → `Other`.
- 2c-3 (IP-native collectors) is a **separate subsystem** — this plan ships 2c-1 +
  2c-2; 2c-3 starts with an investigation spike (Task 3.0) then its own spec.

## File structure

- `golish-agent-kit/src/db_traits/repo.rs` — add `in_scope_typed_assets` trait
  method (default `Ok(vec![])`).
- golish-db `db_traits` impl (the `impl …Repo for GolishDbRepoProvider` block —
  locate via `rg "fn in_scope_assets" backend/crates/golish-db`) — implement the
  typed SELECT.
- `golish-agent-kit/src/harness/gate/rule_engine.rs` — `GateContext.asset_types`;
  authoritative class in `coverage_complete`; unit test.
- `golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs` —
  `fetch_in_scope_typed_assets_for_gate`; build + inject `asset_types`.
- `golish-db/src/repo/coverage_truth.rs` — (2c-2) type-aware `assemble_truth_facts`.

---

## Phase 2c-1 — authoritative type axis

### Task 1.1 — typed in-scope read (TDD: SQL shape)

**File:** golish-db repo (add `fn build_in_scope_typed_assets_sql() -> String`
next to `coverage_truth::build_in_scope_values_sql`, or in the targets repo —
keep it beside the existing in-scope value SQL).

**Step 1 — failing test** (in that module's `mod tests`):
```rust
#[test]
fn in_scope_typed_assets_sql_selects_value_and_type_scope_org() {
    let sql = build_in_scope_typed_assets_sql();
    assert!(sql.contains("value"));
    assert!(sql.contains("target_type::text"));
    assert!(sql.contains("scope::text = 'in'"));
    assert!(sql.contains("($1 IS NULL OR organization_id = $1)"));
}
```
**Step 2 — run, confirm fail:**
`cd backend && cargo nextest run -p golish-db in_scope_typed_assets`
**Step 3 — implement:**
```rust
/// In-scope (value, targets.type) pairs for host-aware coverage classification.
/// $1 IS NULL ⇒ all in-scope (no org filter). DISTINCT to match the gate axis.
fn build_in_scope_typed_assets_sql() -> String {
    "SELECT DISTINCT value, target_type::text FROM targets \
       WHERE scope::text = 'in' \
         AND ($1 IS NULL OR organization_id = $1)"
        .to_string()
}

pub async fn in_scope_typed_assets(
    pool: &PgPool,
    org_id: Option<Uuid>,
) -> Result<Vec<(String, String)>> {
    Ok(sqlx::query_as::<_, (String, String)>(&build_in_scope_typed_assets_sql())
        .bind(org_id)
        .fetch_all(pool)
        .await?)
}
```
**Step 4 — run, confirm pass.** **Commit:** `feat(db): in-scope typed (value,type) read for host-aware coverage`.

### Task 1.2 — `db_traits` trait method

**File:** `golish-agent-kit/src/db_traits/repo.rs` (after `in_scope_targets`, ~176).
**Step 1 — add the method (default empty = fail-safe):**
```rust
/// In-scope (value, targets.type) pairs for an org (None = all in-scope).
/// Powers host-aware coverage's authoritative asset classification. Default
/// empty ⇒ the gate falls back to value-inference (2a/2b behavior).
async fn in_scope_typed_assets(
    &self,
    org_id: Option<Uuid>,
) -> anyhow::Result<Vec<(String, String)>> {
    let _ = org_id;
    Ok(Vec::new())
}
```
**Step 2 — implement in the golish-db provider** (the same impl block that has
`in_scope_assets`): delegate to Task 1.1's `in_scope_typed_assets(pool, org_id)`.
**Step 3 — verify:** `cargo check -p golish-agent-kit -p golish-db`.
**Commit:** `feat(harness): in_scope_typed_assets db_traits method`.

### Task 1.3 — `GateContext.asset_types` + authoritative class (TDD)

**File:** `golish-agent-kit/src/harness/gate/rule_engine.rs`
**Step 1 — failing unit test** (append to `mod tests`, mirror the 2a parity test
at ~1190 for deliverable/ctx construction):
```rust
#[test]
fn host_aware_uses_authoritative_type_over_value() {
    use super::super::types::StageKind;
    // A domain whose VALUE parses as an IP would be mis-dropped by from_value;
    // authoritative type 'domain' must keep the full intel set.
    let techs = [
        "GOLISH-INTEL-DNS", "GOLISH-INTEL-SUBDOMAIN", "GOLISH-INTEL-CT",
        "GOLISH-INTEL-WHOIS", "GOLISH-INTEL-ASN", "GOLISH-INTEL-OSINT",
    ];
    let asset = "1.2.3.4"; // value looks like an IP …
    let facts: Vec<EvidenceFact> = ["GOLISH-INTEL-WHOIS","GOLISH-INTEL-ASN","GOLISH-INTEL-OSINT"]
        .iter().map(|t| EvidenceFact { asset: asset.into(), technique: (*t).into(), outcome: EvidenceOutcome::Found }).collect();
    let mut types = std::collections::HashMap::new();
    types.insert(asset.to_string(), "domain".to_string()); // … but typed domain
    let ctx = GateContext {
        in_scope_assets: Some(vec![asset.into()]),
        asset_types: Some(types),
        expected_techniques: Some(techs.iter().map(|s| s.to_string()).collect()),
        evidence_facts: Some(facts),
    };
    let d = coverage_test_deliverable_empty();         // existing helper (2a test)
    let mut spec = spec_with_expected(&techs);
    spec.kind = StageKind::TargetIntel;
    spec.host_aware_coverage = true;
    // Domain ⇒ all 6 required; only 3 Found ⇒ Block (NOT relaxed like an IP).
    assert!(matches!(
        coverage_complete(&d, &spec, &ctx, None, false, true, false, None, &default_on_fail()),
        GateCheckOutcome::Block { .. }
    ));
}
```
> Reuse the exact deliverable/`OnFail`/helper construction from the 2a
> `host_aware_coverage_relaxes_ip_not_domain` test in this module — do not invent
> new shapes.

**Step 2 — run, confirm fail** (compile error: `asset_types` missing):
`cd backend && cargo nextest run -p golish-agent-kit host_aware_uses_authoritative`
**Step 3 — implement.** Add the field after `in_scope_assets`:
```rust
    pub in_scope_assets: Option<Vec<String>>,
    /// Host-aware coverage 2c: value -> targets.type, for authoritative
    /// per-asset classification. None ⇒ fall back to value inference (2a/2b).
    pub asset_types: Option<std::collections::HashMap<String, String>>,
```
Fix every `GateContext { .. }` literal (add `asset_types: None,`):
`cd backend && rg -n "GateContext \{" crates/golish-agent-kit/src` (the hook at
execute.rs:1960 + each test site). In `coverage_complete`, replace the 2a/2b
`let class = AssetClass::from_value(asset);` with:
```rust
let class = ctx
    .asset_types
    .as_ref()
    .and_then(|m| m.get(asset.as_ref() as &str))
    .map(|ty| crate::harness::technique_resolver::AssetClass::from_target_type(ty))
    .unwrap_or_else(|| crate::harness::technique_resolver::AssetClass::from_value(asset));
```
(`asset` is the loop var used in 2a; match its exact type — deref as needed.)
**Step 4 — run, confirm pass + no regression:**
`cd backend && cargo nextest run -p golish-agent-kit coverage host_aware`
**Commit:** `feat(harness): GateContext.asset_types authoritative classification`.

### Task 1.4 — hook wiring

**File:** `golish-agent-kit/src/task_orchestrator/subtask_phases/execute.rs`
**Step 1 — add `fetch_in_scope_typed_assets_for_gate`** mirroring
`fetch_in_scope_assets_for_gate` (1193) but calling `repo.in_scope_typed_assets(
org_id)` and returning `Option<HashMap<String,String>>` (None when empty/no
stage). **Step 2 — in `apply_harness_gate_hook`** (build site ~1960) add
`asset_types` to the `GateContext { … }`:
```rust
let gate_ctx = crate::harness::GateContext {
    in_scope_assets,
    asset_types,            // NEW
    expected_techniques,
    evidence_facts,
};
```
(compute `asset_types` next to `in_scope_assets`, same org scoping.)
**Step 3 — verify:** `cargo nextest run -p golish-agent-kit` + `cargo check -p golish`.
**Commit:** `feat(harness): inject authoritative asset_types into the coverage gate`.

---

## Phase 2c-2 — type-aware truth projection

### Task 2.1 — `assemble_truth_facts` skips domain-only org facts on IP (TDD)

**File:** `golish-db/src/repo/coverage_truth.rs`
**Step 1 — failing test** (append to `mod tests`):
```rust
#[test]
fn assemble_skips_ct_for_ip_assets() {
    let empty = subs(&[]);
    let mut inputs = empty_inputs(&empty);
    inputs.has_ct = true;          // org has CT data
    inputs.has_asn = true;         // … and ASN
    let assets = vec!["a.com".to_string(), "1.2.3.4".to_string()];
    let types = vec!["domain".to_string(), "ip".to_string()]; // NEW parallel types
    let facts = assemble_truth_facts(&assets, &types, &inputs);
    // domain gets CT; the IP does NOT (CT is domain-only); both get ASN.
    assert!(facts.contains(&("a.com".to_string(), TECH_CT)));
    assert!(!facts.contains(&("1.2.3.4".to_string(), TECH_CT)));
    assert!(facts.contains(&("1.2.3.4".to_string(), TECH_ASN)));
}
```
**Step 2 — run, confirm fail** (signature mismatch):
`cd backend && cargo nextest run -p golish-db assemble_skips_ct_for_ip`
**Step 3 — implement.** Add a `types: &[String]` param (parallel to
`in_scope_assets`) to `assemble_truth_facts`; gate the domain-only org push:
```rust
pub(crate) fn assemble_truth_facts(
    in_scope_assets: &[String],
    types: &[String],
    inputs: &TruthInputs<'_>,
) -> Vec<(String, &'static str)> {
    use crate::harness_class::is_ip_like; // or inline: type == "ip"/"cidr"/"ip_address"
    let mut facts = Vec::new();
    for (i, asset) in in_scope_assets.iter().enumerate() {
        let ip_like = matches!(types.get(i).map(String::as_str),
            Some("ip" | "ipv4" | "ipv6" | "ip_address" | "cidr" | "range" | "netblock"));
        if inputs.has_asn { facts.push((asset.clone(), TECH_ASN)); }
        if inputs.has_ct && !ip_like { facts.push((asset.clone(), TECH_CT)); } // domain-only
        if inputs.has_whois { facts.push((asset.clone(), TECH_WHOIS)); }
        if inputs.has_osint { facts.push((asset.clone(), TECH_OSINT)); }
        if inputs.has_subsidiary { facts.push((asset.clone(), TECH_SUBSIDIARY)); }
        // … per-asset value-set pushes unchanged …
    }
    facts
}
```
(Do **not** add a new module; inline the `ip_like` check. Update the existing
`assemble_*` tests to pass a `types` slice of the right length — same-length
`vec!["domain"; assets.len()]` where type is irrelevant.)
**Step 4 — thread types at the caller** `coverage_truth_facts` (245): it already
has `in_scope_assets`; fetch the parallel types via the Task 1.1
`in_scope_typed_assets` (or accept a `types: &[String]` arg and let the gate hook
pass them). Keep `org_id=None` path working (types may be empty ⇒ treat all as
non-ip = today's behavior, fail-safe toward *keeping* facts).
**Step 5 — run:** `cargo nextest run -p golish-db coverage_truth`.
**Commit:** `feat(db): type-aware truth projection (no CT on IP assets)`.

### Task 2.2 — full verification + flag parity

Run the **mandatory parity test** (design §6): a known mixed domain+IP
`--stage-run target_intel`, gate decisions with 2c-1/2c-2 in vs out — assert
**zero** correctly-typed decision delta (2c-1) and zero gate delta (2c-2; only
ledger shrinks). Capture into `agent-progress.md`. Run scoped
`cargo nextest -p golish-agent-kit -p golish-db` + `cargo check -p golish`. Only
mark passing after green + **user sign-off**.

---

## Phase 2c-3 — IP-native required techniques (separate spec)

> **Do not start as tasks here.** 2c-3 adds a new recon capability (PTR + RIR
> WHOIS collectors), DB `*_values` queries, and new technique ids — a distinct
> subsystem. Per writing-plans, split it.

### Task 3.0 — investigation spike (read-only, output = its own spec)
1. `rg -n "fn .*resolve|dns_records|whois" backend/crates/golish-recon-app/src` —
   find how `dns_records` / `whois` collectors run + persist (the pattern PTR /
   IP-WHOIS must follow).
2. Confirm storage: is there a PTR row shape in `dns_records` (record_type='PTR')?
   an IP-WHOIS column/table? If not, a migration is needed (AGENTS.md §2.7 →
   user sign-off + I10 backward-compat).
3. Write `docs/design/2026-06-15-host-aware-coverage-2c3-ip-native.md` (collectors,
   storage, `coverage_truth` `build_rdns_values_sql`/`build_ipwhois_values_sql`,
   `technique_resolver` baseline+matrix arms `GOLISH-INTEL-RDNS`/`-IPWHOIS`
   required for Ip/Cidr) + its own plan. Then implement under that plan with the
   parity discipline (IPs gain required RDNS/IPWHOIS cells ⇒ ship collectors +
   flag together).

---

## Self-check (writing-plans)

- **Spec coverage:** design §4.1/4.2 → Tasks 1.1–1.4; §4.3 → Tasks 2.1–2.2; §4.4 →
  Task 3.0 (split). §6 parity → Task 2.2 + 3.0.
- **No placeholders for shipped phases:** every 2c-1/2c-2 step has real code or an
  exact `rg`/mirror instruction; 2c-3 is explicitly a spike→own-spec (not faked).
- **Type consistency:** `in_scope_typed_assets(org_id) -> Vec<(String,String)>`,
  `GateContext.asset_types: Option<HashMap<String,String>>`,
  `assemble_truth_facts(assets, types, inputs)`, `AssetClass::from_target_type`,
  `technique_applies` — identical across tasks + design.
- **Fail-safe:** every new field defaults empty/None ⇒ byte-identical when the
  flag is off or types are absent (I10).
