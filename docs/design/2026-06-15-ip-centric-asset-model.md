# 2026-06-15 · IP-centric asset model (targets tree refactor)

> Goal: stop showing domains and IPs as one flat layer under an org; make the
> attack-surface tree **host/IP-centric** — each IP shows the domains that
> resolve to it and the URLs hosted on it. User directive: solve it at the root
> (storage), not a cosmetic view-only patch.
>
> Status: **DESIGN — approved** (2026-06-15): scenario **B now, C deferred to
> Phase 2**; §6 defaults accepted. Implementation plan:
> `docs/superpowers/plans/2026-06-15-ip-centric-asset-model.md`. Evidence below
> was gathered from the live DB (project `Test1`, embedded PG `:15432`) +
> source on 2026-06-15.
>
> **状态更新（2026-06-22 · 核当前代码 + git log）**：✅ **Phase 0 + Phase 1 已落地**。`targets.real_ip` 落点 + 回填（`land_target_intel_coverage`/`recon_backfill_real_ip`）；前端 host/IP-centric 树 `buildHostTree` + `OrgTreeSidebar`（commit `521a39a0` IP-centric target panel）。**Phase 2（host-aware coverage）** 归 2c 系列单独跟踪（见 2c-3 文档：2c-3a 已落、2c-3b 已回退）。

## 0. Problem (evidence)

- `targets` is a **flat list per org**. In `Test1`: 19 `domain` + 24 `ip`
  rows, all `parent_id IS NULL` (0 rows nested). UI renders them in one
  "assets" group (`OrgTreeSidebar` → `node.targets`), domains and IPs
  interleaved.
- The columns to express hierarchy already exist but are **unused**:
  - `targets.parent_id` (self-FK) — 0 rows use it.
  - `targets.real_ip` (resolved IP of a domain) — **0/19 domains populated**.
  - `targets.ports` (JSON) — per-target ports (host model already half-there).
- The domain→IP relationship data partly exists in `dns_records`
  (`A`/`AAAA` rows keyed by the **domain's** `target_id`), but it is **sparse**:
  only 5 A/AAAA edges for 19 domains. And it is genuinely **many-to-many** —
  `pingan.cn` already resolves to 2 IPs (one `A`, one `AAAA`).
- ts-rs `Target` (`golish-app-core/src/domain/targets.rs`) **already exports
  `parent_id` and `real_ip`** to the frontend (`lib/generated/Target.ts`); the
  tree builder just ignores them.

**Conclusion:** this is ~70% a *data-population + view-derivation* problem and
~30% a *modeling* problem. The plumbing (columns, ts-rs fields, dns_records
table) is already in place; what's missing is (a) **persisting the
domain↔IP/URL→host edges during recon**, and (b) **a tree builder that renders
IP-first from those persisted edges**.

## 1. The hard constraint: domain↔IP is many-to-many

A strict "URL belongs to IP" single-parent tree is lossy:

- one domain → N IPs (CDN, round-robin DNS, A + AAAA). Already true here.
- one IP → N domains (shared hosting / vhosts).
- CDN/WAF: the *resolved* IP ≠ the *real* IP (`real_ip`/`cdn_waf` columns hint
  at this).
- some domains have no resolved IP (passive-only); some IPs have no domain
  (found via ASN/CIDR).

So the **source of truth** for the relationship must be an **edge set**, not a
single `parent_id`. A single-parent tree can still be *rendered* on top (pick a
primary IP per domain), but it must not be the only place the relationship
lives.

## 2. Approaches considered

### Option A — `parent_id` single-parent tree (simplest, lossy)
Set `targets.parent_id = <primary IP target>` for each domain; render the tree
straight off `parent_id`.
- ✅ tiny: reuse an existing column; tree builder reads `parent_id`.
- ❌ can't represent a domain on multiple IPs (must pick one / duplicate).
- ❌ loses the M:N truth; future port/service work inherits the lossy model.

### Option B — explicit edge layer + derived IP-centric tree (**recommended**)
Persist the relationship as edges (reuse `dns_records` for domain→IP A/AAAA;
add URL→host), populate them during recon, and **derive** the IP-centric tree
from the edges. Keep domain & IP as first-class `targets`.
- ✅ truthful M:N; matches how recon data actually arrives.
- ✅ most of the storage already exists (`dns_records`); low new surface.
- ✅ tree is stable/persisted, not a render-time heuristic.
- ➖ tree builder is a join (edges × targets), slightly more code than reading
  `parent_id`.
- Optional nicety: also set `parent_id` to the *primary* IP for the common
  single-IP case so the hierarchy is explicit in storage too (denormalized
  cache of "primary edge"), with `dns_records` remaining the M:N truth.

### Option C — full host-centric storage + coverage/gate by IP (biggest)
Everything in B **plus** re-key the `target_intel` coverage/gate truth and
ports/services to be per-IP.
- ✅ end-to-end host model.
- ❌ touches the harness core (`coverage_truth`, gate rules, evidence
  projection) — high risk, IDOR re-review, ts-rs churn across many consumers.
- Defer: do B first; C only if host-keyed coverage is actually needed.

**Recommendation: Option B**, delivered in phases (below). C's coverage rework
is explicitly out of scope for this change and tracked as a follow-up.

## 3. Chosen model (Option B)

### 3.1 Asset entities (unchanged)
`targets` rows stay typed (`domain | subdomain* | ip | cidr | url | wildcard`)
and first-class. *(\*`subdomain` is a `target_assets` concept today; domains and
subdomains both live as `domain`-type targets / `target_assets`.)*

### 3.2 Relationship edges (the new "root")
Reuse and extend the existing edge store rather than invent a new table:

- **domain → IP**: `dns_records` `A`/`AAAA` rows
  (`target_id = domain`, `value = IP`). Already M:N. This is the primary edge.
- **url → host**: a URL target's host (domain or IP) — derived from the URL
  `value`; persisted as the URL target's `parent_id` (a URL has exactly one
  host, so single-parent is correct here).
- **domain → primary IP cache** (optional): `targets.real_ip` (already exists)
  +/or `targets.parent_id` set to the primary IP target for the single-IP case,
  as a denormalized convenience. `dns_records` stays authoritative for M:N.

### 3.3 IP-centric tree (derived, rendered)
New builder produces, per org:

```
org
├─ IP 202.69.26.13
│   ├─ pingan.com.cn        (domain, via dns_records A)
│   └─ https://pingan.com.cn/login  (url, host = pingan.com.cn)
├─ IP 2404:7180:…           (AAAA)
│   └─ pingan.cn
├─ 未解析域名 (no A/AAAA yet)
│   └─ life.pingan.com
└─ (bare IPs with no domain shown as leaf IP nodes)
```

- A domain on N IPs appears under each (or under its primary IP with a "+N"
  badge — UI decision, §6 open Q).
- Unresolved domains → "未解析" bucket (so nothing is hidden — AGENTS.md I8
  "checked-empty ≠ unchecked": an unresolved domain is *unchecked*, not empty).
- Toggle: **按类型 / 按 IP** so the old flat view is still available.

## 4. Phasing

- **Phase 0 — populate resolution edges (backend, the real root fix).**
  Recon must resolve in-scope domains and persist domain→IP into `dns_records`
  *and* set `real_ip`. Without this the IP view is empty (today 0/19). Most of
  the path exists (`dns_records::upsert`, `targets::update_recon_extended_by_id`
  sets `real_ip`); wire it so the `target_intel` DNS step always lands A/AAAA
  for every in-scope domain target, and backfill existing rows once.
- **Phase 1 — IP-centric tree builder + toggle (frontend).**
  New `buildHostTree` in `lib/target-panel/` from `targets` + `dns_records`
  edges (expose the edges via an API/derived field); `OrgTreeSidebar` renders
  host nodes; "按类型/按 IP" toggle. No schema change.
- **Phase 2 — (deferred) host-keyed coverage/ports.** Not in this change.

## 5. Touch points (files)

Backend:
- `golish-db/src/repo/dns_records.rs` — already has `upsert` + a presence read;
  add a read that returns `(domain_value, ip_value)` edges per org for the tree.
- `golish-recon-app/src/organization_recon/persistence.rs` — DNS landing
  (`land_target_intel_coverage` / dns landing) must populate edges for every
  in-scope domain, and set `real_ip`.
- `golish-app-core/src/domain/targets.rs` — ts-rs `Target` (already has
  `parent_id`/`real_ip`; no change unless we surface an `edges` field).
- New: an IPC read (`recon_list_asset_edges` or extend the targets list
  response) so the frontend has the domain↔IP edges. Route via
  `commands_facade/<domain>.rs` + `frontend/lib/api/...` (AGENTS.md M1/M2).
- ts-rs: any new wire type derives `#[ts(...)]` (I5).

Frontend:
- `lib/target-panel/org-tree.ts` — add `buildHostTree(orgs, targets, edges)`
  alongside `buildOrgTree`; keep `buildOrgTree` for the "by type" view.
- `components/TargetPanel/OrgTreeSidebar.tsx` + `TargetTreeRow.tsx` — render
  host (IP) nodes with nested domains/URLs; add the view toggle.
- `lib/api/targets.ts` (or a new `asset-edges` API) — fetch the edges.

## 6. Open questions for review (please confirm before the plan)

1. **Scenario depth** — Option B (recommended) vs Option C (also re-key
   coverage/gate by IP). Default: **B**.
2. **Multi-IP rendering** — a domain on N IPs: show it under *each* IP, or under
   a single *primary* IP with a "+N more" badge? Default: **primary IP + badge**
   (avoids visual duplication; `dns_records` keeps full truth).
3. **Where do URLs sit** — under their domain, or directly under the IP?
   Default: **under the domain** (url.parent_id = domain), and the domain under
   the IP — i.e. `IP → domain → url`. Bare-IP URLs go directly under the IP.
4. **Resolution scope (Phase 0)** — resolve only in-scope domains, or all?
   Default: **in-scope only** (cost + scope discipline).
5. **Backfill** — run a one-off resolve for existing domains, or only resolve
   going forward? Default: **backfill once** so the view isn't empty for the
   current `Test1` data.

## 7. Risks / non-goals

- Non-goal: changing `coverage_truth` / gate semantics (Phase 2 only).
- Risk: M:N rendered as a tree is inherently a projection — we keep
  `dns_records` as the truth and treat the tree as a view to avoid data loss.
- Risk: Phase 0 adds active resolution; must respect scope/authorization (only
  in-scope, passive DNS is fine; no port scan here). Keep it within the existing
  `target_intel` recon step.
