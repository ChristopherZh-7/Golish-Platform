# 2026-06-15 · Host-aware coverage 2c-3 — IP-native required techniques (RDNS + IP-WHOIS)

> **Sub-phase 2c-3** of host-aware coverage, split out per the 2c plan (its own
> spec because it adds a new recon collector subsystem + DB storage). Builds on
> 2a (`342eb54a`), 2b (`e12a7638`), 2c plan (`70615589`), 2c-1 (`a409d732`),
> 2c-2 (`fae164e1`).
>
> Status: **DESIGN — output of the Task 3.0 investigation spike. Needs user
> sign-off before implementation** (AGENTS.md I7/I8 harness-core + §2.7 DB
> migration + new active network collectors). No code written. Evidence below was
> read from source on 2026-06-15 (spike report).
>
> **状态更新（2026-06-22 · 核当前代码 + git log）**：🟡 **2c-3a 已落地，2c-3b 已回退**。2c-3a 采集器 `land_rdns`/`land_ip_whois` + DB 存储/truth：commits `f140df06`（storage + truth）+ `83c45c0d`（collectors）。**2c-3b 实现后被回退**：`a3bb618c`（require RDNS + IP-WHOIS for IP assets）→ `b2f5c2d2`（revert，原因 stage_run regression A/B）。故上面「No code written」已过时；**2c-3b 当前不在生效**，重启 gate 翻转需先解决该 regression + 用户 sign-off。

## 0. One sentence

2a/2b/2c-1/2c-2 stopped IPs from being *falsely* asked for domain-only techniques
and cleaned the ledger; 2c-3 gives an in-scope IP its *own* positive coverage —
**reverse DNS (PTR)** and **RIR/netblock IP-WHOIS** — so an IP is required to be
actually investigated, not merely excused from domain checks.

## 1. Evidence (current state)

The passive-intel layer has a **split personality** (spike §4):

- **Org-level collectors** → `organizations.*` columns → `has_*` bools → stamped
  on every asset: CT (`organizations.certificates`), domain WHOIS
  (`organizations.whois`, RDAP), ASN (`organizations.asns`, Team Cymru), OSINT.
- **Per-asset collectors** → `dns_records` / `target_assets` → `*_values`
  HashSets → stamped only on the matching `targets.value`: forward DNS
  (`land_dns_records`, `persistence.rs:323`), subdomain.

Key facts for 2c-3:

1. **`dns_records.record_type` is free `TEXT`** (`migrations/20260612000001_dns_records.sql:9`),
   and the migration comment already lists `PTR`. So PTR rows need **no schema
   change** — but they must attach to the **IP target's** `target_id`
   (`targets.value='1.2.3.4'`, `target_type='ip'`), because coverage keys off
   `t.value` (`coverage_truth::build_in_scope_values_sql:97`,
   `dns_records::present_target_values`).
2. **`land_dns_records` only resolves `target_type='domain'`** targets and only
   does forward A/AAAA (`persistence.rs:323-405`) — there is **no reverse path**
   for IP targets today.
3. **No IP-WHOIS / RIR store exists** (spike §2): `organizations.whois` is
   *domain* RDAP (org-level, wrong granularity); `organizations.asns` (Team Cymru
   `whois.cymru.com:43`, `asset_intel/service/enrich.rs:5`) is IP→ASN only and
   org-level. A grep for `rdap.org/ip` / `ip_whois` found **zero** matches.
4. **The collector "registry" is 4 parallel mechanisms** (spike §4); the cleanest
   insertion point for new built-in collectors is the post-commit hook
   `land_target_intel_coverage` (`persistence.rs:177`), mirroring `land_dns_records`
   / `land_ct_and_whois`.
5. After 2a, `technique_applies(TargetIntel, Ip, …)` keeps only WHOIS/ASN/OSINT
   (all org-level) — so an IP currently completes `target_intel` with **zero
   IP-specific collection** (`technique_resolver.rs:120-127`).

## 2. Goal / scope / non-goals

- **In scope:** two new IP/CIDR-required intel techniques:
  - `GOLISH-INTEL-RDNS` — reverse DNS / PTR for the IP.
  - `GOLISH-INTEL-IPWHOIS` — RIR/netblock WHOIS for the IP (netname, org, country,
    abuse contact) — distinct from domain WHOIS and from ASN.
  Plus their collectors, storage, `coverage_truth` per-asset queries, baseline +
  matrix, and the gated rollout.
- **Non-goals:** changing domain-side techniques; the org-level WHOIS/ASN
  semantics; 2b flag flips; any active scanning beyond passive PTR/RDAP lookups.

## 3. Storage decisions (spike §5)

### 3.1 PTR → reuse `dns_records` (no migration)

Row shape (additive; `record_type` is TEXT):

```
target_id   = UUID of the IP target  (targets.value='1.2.3.4', target_type='ip')
record_type = 'PTR'
name        = '1.2.3.4'              (the queried IP; pick one convention, document it)
value       = resolved hostname      (e.g. 'host.example.com')
source      = 'resolver'
```

No enum/schema change. New presence query keyed on PTR + IP type (§5.2).

### 3.2 IP-WHOIS → new per-asset column `targets.ip_whois JSONB` (migration, §2.7)

Org-level `organizations.whois` is the wrong granularity (can't say *which* IP).
A per-asset JSONB column mirrors `organizations.whois`'s shape at asset
granularity and matches the per-asset coverage pattern. Decision: **option A**
(column on `targets`) over a new table (option B) — least churn, mirrors
`targets.real_ip`/recon-extended columns.

```sql
-- migration YYYYMMDD_targets_ip_whois.sql (expand-first, I10: nullable, no backfill)
ALTER TABLE targets ADD COLUMN IF NOT EXISTS ip_whois JSONB;
-- reads treat NULL / 'null' / '{}' as empty (shape-agnostic, like has_whois)
-- Suggested shape: { netname, org, country, cidr, abuse, source:'rdap'|'rir', raw_ref }
```

**This migration requires explicit user sign-off (§2.7) before applying.**

## 4. Collectors (mirror existing landing hooks)

Both land via the post-commit hook `land_target_intel_coverage` (`persistence.rs:177`),
alongside `land_dns_records`/`land_ct_and_whois`. Both are **passive** network
lookups; respect scope (in-scope IP/CIDR targets only) + per-host timeout (mirror
`land_dns_records`'s 3s `tokio::time::timeout`).

### 4.1 `land_rdns` (reverse DNS)

For each in-scope `target_type IN (ip,cidr,…)` with no PTR row yet:
reverse-resolve (`dns_lookup`/`tokio` reverse, or `dig -x`). Upsert a `'PTR'`
`dns_records` row keyed to that IP target (§3.1) via the existing
`dns_records::upsert`. CIDR handling: resolve representative host(s) or treat the
CIDR's coverage as "attempted" once any PTR lands — **decide in the plan**
(default: per-listed-IP; a bare CIDR with no enumerated hosts is `checked_empty`
if the lookup runs and yields nothing — I8: attempted-empty ≠ unchecked).

### 4.2 `land_ip_whois` (RIR WHOIS)

For each in-scope IP/CIDR with empty `ip_whois`: query `https://rdap.org/ip/{ip}`
(symmetric with the domain RDAP in `land_ct_and_whois:454`), extract
netname/org/country/cidr/abuse, write `targets.ip_whois` via a new
`targets::set_ip_whois_by_id` (mirror `set_real_ip_by_id`). Fall back to RIR
whois (`whois -h whois.{arin,ripe,apnic}.net`) only if RDAP is insufficient
(plan decision).

## 5. Coverage truth + gate wiring (spike §6)

### 5.1 New technique ids (`coverage_truth.rs`)

```rust
pub const TECH_RDNS: &str = "GOLISH-INTEL-RDNS";
pub const TECH_IPWHOIS: &str = "GOLISH-INTEL-IPWHOIS";
```

### 5.2 Per-asset presence queries

```rust
// PTR: reuse dns_records, filter record_type + IP-type assets.
fn build_rdns_values_sql() -> String { /* JOIN dns_records dr ... dr.record_type='PTR'
   AND t.target_type::text IN ('ip','ipv4','ipv6','ip_address','cidr','range','netblock') */ }
// IP-WHOIS: targets.ip_whois non-empty + IP-type assets (shape-agnostic empty check).
fn build_ipwhois_values_sql() -> String { /* t.ip_whois IS NOT NULL AND <> 'null' AND <> '{}'
   AND t.target_type::text IN (...) */ }
```

(Both follow `build_in_scope_values_sql`'s scope+org template; additive — I10.)

### 5.3 `TruthInputs` + `assemble_truth_facts_typed` + `coverage_truth_facts`

- Add `rdns_values`, `ipwhois_values: &HashSet<String>` to `TruthInputs`.
- In `assemble_truth_facts_typed`, add two per-asset pushes (set-membership), like
  `subdomain_values`/`dns_values` — **no `ip_like` gate needed** (they're inherently
  IP-only via the SQL filter; pushing them is harmless on a domain that has none).
- In `coverage_truth_facts`, `fetch_values` the two new queries.

### 5.4 `technique_resolver` (the matrix change — the actual gate behavior)

- `stage_baseline(TargetIntel)` += `GOLISH-INTEL-RDNS`, `GOLISH-INTEL-IPWHOIS`.
- `technique_applies(TargetIntel, …)`:
  ```rust
  "GOLISH-INTEL-RDNS" | "GOLISH-INTEL-IPWHOIS" => matches!(class, Ip | Cidr),
  ```
  i.e. domain/url **drop** them (mirror of SUBDOMAIN/DNS being domain-only);
  IP/CIDR **require** them.
- `resources/harness/stages/target_intel.json` / `technique_taxonomy.json`: declare
  the two ids so the stage's `expected_techniques` headline + taxonomy know them.

### 5.5 Hints + evidence (so the agent can satisfy the new cells)

- `refiner.rs::passive_intel_command_hint`: map RDNS→`dig -x <ip>` / IPWHOIS→
  `whois <ip>` (RIR) so a blocked IP gets an actionable command.
- `harness/evidence_facts.rs::passive_intel_facts_from_command`: tag `dig -x`→RDNS,
  IP `whois`→IPWHOIS, so agent-run commands also satisfy the cells (not only the
  built-in landing).

## 6. Rollout, risk, parity (I7/I8 + §2.7)

- **Behavior change is real and BLOCKING**: adding RDNS/IPWHOIS to the IP baseline
  means in-scope IPs that lack them will **BLOCK** until the collectors run. So
  2c-3 must ship **collectors + truth queries + baseline together** (unlike 2c-1/2c-2,
  which were zero-/nil-change). Sequence within 2c-3: land the collectors + storage
  first (so data can exist), then flip the baseline.
- **Migration** (`targets.ip_whois`): expand-first (nullable), backward-compatible
  (I10); **needs §2.7 sign-off**.
- **Parity test (design 2c §6)**: on a known mixed domain+IP `--stage-run`, the
  only allowed gate delta is IPs gaining RDNS/IPWHOIS *required* cells (PASS once
  the collectors populate them; BLOCK before) — **no domain decision changes**.
- **Network-action risk**: PTR + RDAP/RIR WHOIS are outbound lookups; restrict to
  in-scope IP/CIDR, add timeouts + a "no new resolution if present" guard (mirror
  `land_dns_records`/`land_ct_and_whois`'s "only when missing" checks).
- **Explicit user sign-off before merge** (harness baseline change + DB migration +
  new active collectors).

## 7. Touch points (files)

**No migration (PTR via `dns_records`):**
- `golish-recon-app/src/organization_recon/persistence.rs` — `land_rdns` + wire into `land_target_intel_coverage`.
- `golish-db/src/repo/dns_records.rs` — PTR presence query (or a filtered variant of `present_target_values`).
- `golish-pentest/src/output_store/dns_records.rs` — fix PTR target_id resolution (key by IP, not the arpa `name`) for the `dig -x` tool path.

**Migration required (IP-WHOIS):**
- `golish-db/migrations/YYYYMMDD_targets_ip_whois.sql` — `ALTER TABLE targets ADD COLUMN IF NOT EXISTS ip_whois JSONB`.
- `golish-db/src/repo/targets.rs` — `set_ip_whois_by_id` (mirror `set_real_ip_by_id`).
- `golish-recon-app/src/organization_recon/persistence.rs` — `land_ip_whois` (RDAP `/ip/`).

**Gate + truth (both):**
- `golish-db/src/repo/coverage_truth.rs` — `TECH_RDNS`/`TECH_IPWHOIS`, `build_rdns_values_sql`/`build_ipwhois_values_sql`, `TruthInputs` + `assemble_truth_facts_typed` + `coverage_truth_facts`.
- `golish-agent-kit/src/harness/technique_resolver.rs` — baseline + matrix arm.
- `resources/harness/stages/target_intel.json`, `resources/harness/technique_taxonomy.json`.
- `golish-agent-kit/src/task_orchestrator/refiner.rs`, `harness/evidence_facts.rs` — hints + evidence tagging.
- Tests in `coverage_truth.rs`, `technique_resolver.rs`, `persistence.rs`.

## 8. Open questions (resolve in the plan)

1. **CIDR coverage:** require RDNS/IPWHOIS per enumerated host, or treat a bare
   CIDR as attempted-once? Default: per-IP target; bare CIDR = `checked_empty` when
   the lookup runs empty (I8).
2. **IP-WHOIS source:** RDAP (`rdap.org/ip`) only, or RIR whois fallback? Default:
   RDAP first (symmetric with domain), RIR fallback later.
3. **PTR `name` convention:** store the dotted IP or the `in-addr.arpa` form?
   Default: dotted IP (matches `t.value`), document it.
4. **`ip_whois` store:** column on `targets` (chosen) vs new table — confirm at
   sign-off (column is simpler; a table is better if we later store multiple
   RIR records per IP).

## 9. Phasing (within 2c-3, for the plan)

- **2c-3a (storage + collectors, no gate change):** migration + `set_ip_whois_by_id`
  + `land_rdns` + `land_ip_whois` + `coverage_truth` queries/inputs (data lands,
  but baseline unchanged ⇒ inert). TDD on SQL-shape + assemble + a landing test.
- **2c-3b (gate activation):** `technique_resolver` baseline + matrix + stage JSON
  + hints/evidence + the mandatory parity test. **Sign-off gate.**

## 10. Self-check

- Spec coverage: spike §1→§1; §5 storage gap→§3; §4 collectors→§4; §6 mapping→§5;
  §2.7/I10→§6/§3.2.
- Type/name consistency: `TECH_RDNS`/`TECH_IPWHOIS`, `targets.ip_whois`,
  `set_ip_whois_by_id`, `build_rdns_values_sql`/`build_ipwhois_values_sql`,
  `rdns_values`/`ipwhois_values`, `technique_applies(... Ip|Cidr)` used identically.
- Reuse over new: PTR reuses `dns_records` (no migration); only IP-WHOIS adds a
  column. Collectors reuse the `land_*` hook pattern + RDAP precedent.
