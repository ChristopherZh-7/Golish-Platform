# Host-aware coverage 2c-3 — IP-native techniques implementation plan

> **For the AI worker:** execute task-by-task with `.cursor/skills/executing-plans`
> (TDD, frequent commits). Spec: `docs/design/2026-06-15-host-aware-coverage-2c3-ip-native.md`.
> Assumes 2a/2b/2c-1/2c-2 landed (`342eb54a`,`e12a7638`,`a409d732`,`fae164e1`).
> **BLOCKED ON USER SIGN-OFF**: this adds a DB migration (§2.7), new active
> network collectors, and a harness baseline change that BLOCKs IPs until data
> lands (I7/I8). Do not start 2c-3b (gate activation) without sign-off.

**Goal:** Give in-scope IP/CIDR assets two required passive-intel techniques —
reverse DNS (`GOLISH-INTEL-RDNS`) and RIR IP-WHOIS (`GOLISH-INTEL-IPWHOIS`) — with
collectors, storage, coverage-truth queries, and the gated baseline.
**Architecture:** PTR reuses `dns_records` (TEXT record_type, no migration), keyed
to the IP target; IP-WHOIS adds `targets.ip_whois JSONB` (expand-first migration).
Collectors land via the `land_target_intel_coverage` hook (mirror
`land_dns_records`/`land_ct_and_whois`). Coverage truth gets two per-asset
`*_values` queries; `technique_resolver` requires them for `Ip|Cidr` only.
**Stack:** Rust (`golish-recon-app`, `golish-db`, `golish-agent-kit`), Postgres
migration, JSON stage spec.

## Decisions locked (spec §8 defaults)

- PTR: reuse `dns_records` record_type='PTR', keyed to the IP target's `target_id`,
  `name`=dotted IP, `value`=hostname. No schema change.
- IP-WHOIS: `targets.ip_whois JSONB` (nullable, expand-first). RDAP `rdap.org/ip`
  first; RIR whois fallback deferred.
- CIDR: per-IP target; a bare CIDR whose lookup runs empty = `checked_empty` (I8).
- Both gated by `host_aware_coverage` (already on for target_intel); baseline flip
  is 2c-3b (sign-off).

---

## Phase 2c-3a — storage + collectors + truth (inert: no baseline change)

### Task A.1 — migration: `targets.ip_whois` (§2.7 sign-off first)

**File:** `backend/crates/golish-db/migrations/<next-ts>_targets_ip_whois.sql`
```sql
-- Host-aware coverage 2c-3: per-IP RIR/netblock WHOIS (RDAP /ip/). Nullable,
-- expand-first (I10); reads treat NULL/'null'/'{}' as empty.
-- Shape: { netname, org, country, cidr, abuse, source, raw_ref }
ALTER TABLE targets ADD COLUMN IF NOT EXISTS ip_whois JSONB;
```
**Verify:** `cargo check -p golish-db` (sqlx offline ok); migration applies on a
fresh DB (`just dev` boot or the test harness). **Commit:** `feat(db): targets.ip_whois column (host-aware 2c-3)`.

### Task A.2 — repo: `set_ip_whois_by_id` (TDD: SQL shape)

**File:** `golish-db/src/repo/targets.rs` (mirror `set_real_ip_by_id`).
**Step 1 — failing test:**
```rust
#[test]
fn set_ip_whois_sql_targets_ip_whois_column_by_id() {
    let sql = build_set_ip_whois_sql();
    assert!(sql.contains("UPDATE targets SET ip_whois ="));
    assert!(sql.contains("WHERE id ="));
}
```
**Step 2 — run:** `cargo nextest run -p golish-db set_ip_whois` (fail).
**Step 3 — implement:**
```rust
fn build_set_ip_whois_sql() -> String {
    "UPDATE targets SET ip_whois = $1, updated_at = NOW() WHERE id = $2".to_string()
}
/// Set a target's IP-WHOIS (RIR/netblock) JSON by id. Idempotent overwrite.
pub async fn set_ip_whois_by_id(pool: &PgPool, id: Uuid, ip_whois: &serde_json::Value) -> Result<()> {
    sqlx::query(&build_set_ip_whois_sql()).bind(ip_whois).bind(id).execute(pool).await?;
    Ok(())
}
```
**Commit:** `feat(db): targets set_ip_whois_by_id setter`.

### Task A.3 — coverage_truth: RDNS + IP-WHOIS presence queries + inputs (TDD)

**File:** `golish-db/src/repo/coverage_truth.rs`
**Step 1 — failing SQL-shape tests** (append to `mod tests`):
```rust
#[test]
fn rdns_values_sql_filters_ptr_and_ip_types() {
    let sql = build_rdns_values_sql();
    assert!(sql.contains("dr.record_type = 'PTR'"));
    assert!(sql.contains("JOIN dns_records dr ON dr.target_id = t.id"));
    assert!(sql.contains("t.target_type::text IN"));
    assert!(sql.contains("t.scope::text = 'in'"));
}
#[test]
fn ipwhois_values_sql_filters_nonempty_and_ip_types() {
    let sql = build_ipwhois_values_sql();
    assert!(sql.contains("t.ip_whois IS NOT NULL"));
    assert!(sql.contains("t.ip_whois <> '{}'::jsonb"));
    assert!(sql.contains("t.target_type::text IN"));
}
#[test]
fn assemble_projects_rdns_and_ipwhois_per_asset() {
    let empty = subs(&[]);
    let rdns = subs(&["1.2.3.4"]);
    let ipw = subs(&["1.2.3.4"]);
    let mut inputs = empty_inputs(&empty);
    inputs.rdns_values = &rdns;
    inputs.ipwhois_values = &ipw;
    let assets = vec!["a.com".to_string(), "1.2.3.4".to_string()];
    let types = vec!["domain".to_string(), "ip".to_string()];
    let facts = assemble_truth_facts_typed(&assets, &types, &inputs);
    assert!(facts.contains(&("1.2.3.4".to_string(), TECH_RDNS)));
    assert!(facts.contains(&("1.2.3.4".to_string(), TECH_IPWHOIS)));
    assert!(!facts.iter().any(|(a, t)| a == "a.com" && (*t == TECH_RDNS || *t == TECH_IPWHOIS)));
}
```
**Step 2 — run:** `cargo nextest run -p golish-db coverage_truth` (fail: missing
consts/fields/fns).
**Step 3 — implement:**
- consts: `pub const TECH_RDNS = "GOLISH-INTEL-RDNS"; pub const TECH_IPWHOIS = "GOLISH-INTEL-IPWHOIS";`
- `const IP_TYPES_SQL: &str = "('ip','ipv4','ipv6','ip_address','cidr','range','netblock')";`
- `build_rdns_values_sql()` = `build_in_scope_values_sql("JOIN dns_records dr ON dr.target_id = t.id", &format!("AND dr.record_type = 'PTR' AND t.target_type::text IN {IP_TYPES_SQL}"))`.
- `build_ipwhois_values_sql()` = `build_in_scope_values_sql("", &format!("AND {} AND t.target_type::text IN {IP_TYPES_SQL}", jsonb_non_empty("t.ip_whois")))` (note: `jsonb_non_empty` takes a col expr; pass `"t.ip_whois"`).
- `TruthInputs`: add `pub rdns_values: &'a HashSet<String>, pub ipwhois_values: &'a HashSet<String>,` (+ update `empty_inputs` test helper).
- `assemble_truth_facts_typed`: after the dns push, add `if inputs.rdns_values.contains(asset) { facts.push((asset.clone(), TECH_RDNS)); }` and same for ipwhois (no ip_like gate — the SQL already restricts to IP types).
- `coverage_truth_facts`: `let rdns_values = fetch_values(pool, &build_rdns_values_sql(), org_id).await?; let ipwhois_values = fetch_values(pool, &build_ipwhois_values_sql(), org_id).await?;` and add to the `TruthInputs { .. }`.
**Step 4 — run** (green) + the existing `assemble_combines_all_dimensions_in_stable_order`
test will need the two new fields in its `TruthInputs { .. }` literal (and the
expected output if it sets them) — update it.
**Commit:** `feat(db): coverage_truth RDNS + IP-WHOIS per-asset truth`.

### Task A.4 — collectors `land_rdns` + `land_ip_whois` (mirror existing hooks)

**File:** `golish-recon-app/src/organization_recon/persistence.rs`
Read `land_dns_records` (323-405) and `land_ct_and_whois` (454-575) first; mirror
their structure (in-scope query, "only when missing" guard, 3s timeout, upsert).
- `land_rdns`: select in-scope `target_type IN (ip,…)` with no `'PTR'` `dns_records`
  row; reverse-resolve each (`tokio` reverse lookup or `dns-lookup`); `dns_records::upsert(pool, target_id, project_path, "PTR", &ip, &hostname, "resolver")`.
- `land_ip_whois`: select in-scope IP/CIDR with empty `ip_whois`; GET
  `https://rdap.org/ip/{ip}` (mirror the RDAP call in `land_ct_and_whois`);
  extract netname/org/country/cidr/abuse → `targets::set_ip_whois_by_id`.
- Wire both into `land_target_intel_coverage` (177) + extend `CoverageLandingSummary`.
**Verify:** `cargo nextest run -p golish-recon-app organization_recon` (existing
green; add a unit test for the in-scope IP selection SQL if practical — network
calls stay integration/manual). Manual: run recon on a project with an in-scope IP,
confirm a `'PTR'` `dns_records` row + non-null `targets.ip_whois`.
**Commit:** `feat(recon): land reverse-DNS + IP-WHOIS for in-scope IP assets`.

> After A.1–A.4, data lands but the **baseline is unchanged** ⇒ the gate does not
> yet require RDNS/IPWHOIS (inert). Verify `cargo nextest -p golish-agent-kit` +
> `cargo check -p golish-agent-app` still green.

---

## Phase 2c-3b — gate activation (SIGN-OFF GATE)

### Task B.1 — technique_resolver baseline + matrix (TDD)

**File:** `golish-agent-kit/src/harness/technique_resolver.rs`
**Step 1 — failing tests:**
```rust
#[test]
fn target_intel_requires_ip_native_for_ip_not_domain() {
    let ip = techniques_for(StageKind::TargetIntel, AssetClass::Ip);
    assert!(ip.contains(&"GOLISH-INTEL-RDNS".to_string()));
    assert!(ip.contains(&"GOLISH-INTEL-IPWHOIS".to_string()));
    let dom = techniques_for(StageKind::TargetIntel, AssetClass::Domain);
    assert!(!dom.contains(&"GOLISH-INTEL-RDNS".to_string()));
    assert!(!dom.contains(&"GOLISH-INTEL-IPWHOIS".to_string()));
}
```
**Step 2 — run** (fail). **Step 3 — implement:**
- `stage_baseline(TargetIntel)` += `"GOLISH-INTEL-RDNS", "GOLISH-INTEL-IPWHOIS"`.
- `technique_applies(TargetIntel, …)` add arm: `"GOLISH-INTEL-RDNS" | "GOLISH-INTEL-IPWHOIS" => matches!(class, Ip | Cidr),`.
- Update `target_intel_returns_all_intel_techniques` (now 8 for domain? NO — domain
  drops the 2 IP-native, so `techniques_for(Domain).len()` stays 6; `stage_baseline`
  is now 8, but the matrix filters domain back to 6 and IP to WHOIS/ASN/OSINT+RDNS+IPWHOIS=5).
  Re-derive expected counts in the existing tests and fix.
**Step 4 — run** (green). **Commit:** `feat(harness): require RDNS + IP-WHOIS for IP assets in target_intel (2c-3b)`.

### Task B.2 — stage spec + taxonomy + hints + evidence

- `resources/harness/stages/target_intel.json`: ensure `expected_techniques` /
  `authoritative_techniques` include the two ids (so the headline + authoritative
  set know them); JSON-validate.
- `resources/harness/technique_taxonomy.json`: add the two ids.
- `golish-agent-kit/src/task_orchestrator/refiner.rs::passive_intel_command_hint`:
  RDNS→`dig -x <ip>`, IPWHOIS→`whois <ip>`.
- `golish-agent-kit/src/harness/evidence_facts.rs::passive_intel_facts_from_command`:
  tag `dig -x`→RDNS, IP `whois`→IPWHOIS.
**Verify:** `cargo nextest -p golish-agent-kit`; `python3 -m json.tool` on both JSONs.
**Commit:** `feat(harness): RDNS/IP-WHOIS taxonomy + hints + evidence tagging`.

### Task B.3 — parity + full verification (mandatory, design 2c §6)

Run a known mixed domain+IP `--stage-run target_intel` before/after 2c-3b; assert
the only delta is IPs gaining RDNS/IPWHOIS required cells (BLOCK until collectors
populate, PASS after), **no domain decision changes**. Capture into
`agent-progress.md`. `just precommit` (when disk/env allow). Update `feature_list.json`
2c entry → mark 2c-3 done only after green + sign-off.
**Commit:** `feat(harness): enable IP-native intel coverage (2c-3 complete)`.

---

## Self-check (writing-plans)

- Spec coverage: spec §3.1→A.3(PTR query); §3.2→A.1+A.2; §4→A.4; §5→A.3+B.1+B.2;
  §6 parity→B.3; §9 phasing→2c-3a/2c-3b split.
- No placeholders for shipped steps: A.1–A.3 + B.1 have real code/SQL; A.4
  (collectors w/ network I/O) gives exact mirror targets + upsert calls (the
  network specifics are read-then-mirror, not invented).
- Type/name consistency: `TECH_RDNS`/`TECH_IPWHOIS`, `targets.ip_whois`,
  `set_ip_whois_by_id`, `build_rdns_values_sql`/`build_ipwhois_values_sql`,
  `rdns_values`/`ipwhois_values`, `technique_applies(... Ip|Cidr)` identical to spec.
- Fail-safe / I10: migration nullable expand-first; new queries additive; baseline
  flip isolated to 2c-3b behind sign-off.
