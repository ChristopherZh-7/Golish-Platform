# IP-centric asset model — implementation plan

> **For the AI worker:** execute task-by-task with `.cursor/skills/executing-plans`
> (TDD, frequent commits). Spec: `docs/design/2026-06-15-ip-centric-asset-model.md`.

**Goal:** Render the target tree host/IP-centric (each IP shows the domains that
resolve to it + URLs), driven by persisted resolution data — without touching
the coverage/gate truth (that is Phase 2, deferred).
**Architecture:** Option B. Keep `targets` flat + typed; persist the primary
resolved IP on `targets.real_ip` (Phase 0); derive an IP-grouped tree on the
frontend from `real_ip` + a "by type / by IP" toggle (Phase 1). `dns_records`
stays the M:N edge truth. No schema migration.
**Stack:** Rust (`golish-recon-app`, `golish-db`), React/TS (`frontend`),
ts-rs (`Target` already exports `real_ip`/`parent_id`).

---

## Decisions locked (from design review)

- Scenario **B** now; **C (host-aware coverage) deferred to Phase 2** — §Phase 2.
- Multi-IP domain → grouped under its **primary IP** (`real_ip`); full M:N stays
  in `dns_records` (a "+N IPs" badge is a Phase 1.5 nicety, not required).
- Hierarchy: `org → IP → domain` (+ URLs under their domain). Bare IPs = leaves.
- Unresolved domains (`real_ip == ''`) → an "未解析" bucket per org (I8: unchecked
  ≠ empty).
- Resolve **in-scope** domains only; **backfill once** from existing
  `dns_records` so the current `Test1` data isn't empty.

## File structure

Backend:
- `golish-db/src/repo/targets.rs` — add `set_real_ip_by_id` + `backfill_real_ip_from_dns`.
- `golish-recon-app/src/organization_recon/persistence.rs` — `land_dns_records`
  also sets `real_ip` (primary A record).
- `golish-recon-app/src/targets/cmds.rs` (or the recon command facade) — expose a
  `recon_backfill_real_ip` command for the one-off + manual refresh.
- Command wiring per AGENTS.md M1 (facade) — confirm exact facade file during 1.x.

Frontend:
- `frontend/lib/target-panel/org-tree.ts` — `OrgTreeNode` gains `kind`; add
  `buildHostTree`.
- `frontend/lib/target-panel/host-tree.test.ts` — vitest for `buildHostTree`.
- `frontend/components/TargetPanel/OrgTreeSidebar.tsx` — render `kind: "host"`
  nodes (Network icon, no org-action buttons).
- `frontend/components/TargetPanel/TargetGroupedView.tsx` — `viewMode` state +
  toggle; pick builder.
- `frontend/lib/i18n/{en,zh-CN}.json` — toggle + bucket labels.

---

## Phase 0 — persist the primary resolved IP (backend)

### Task 0.1 — repo: `set_real_ip_by_id` (TDD: SQL shape)
**File:** `backend/crates/golish-db/src/repo/targets.rs`
Add:
```rust
/// Set a target's primary resolved IP (`real_ip`) by id. No project scope —
/// caller owns the id (recon landing). Idempotent: re-running overwrites.
pub async fn set_real_ip_by_id(pool: &PgPool, id: Uuid, real_ip: &str) -> Result<()> {
    sqlx::query("UPDATE targets SET real_ip = $1, updated_at = NOW() WHERE id = $2")
        .bind(real_ip)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
```
And a backfill that needs **no new resolution** (derives from existing `dns_records`):
```rust
fn build_backfill_real_ip_sql() -> String {
    "UPDATE targets t SET real_ip = sub.ip, updated_at = NOW() \
       FROM (SELECT DISTINCT ON (target_id) target_id, value AS ip \
               FROM dns_records WHERE record_type = 'A' \
               ORDER BY target_id, created_at) sub \
      WHERE t.id = sub.target_id AND t.real_ip = '' \
        AND ($1 IS NULL OR t.project_path = $1)".to_string()
}

/// Backfill `real_ip` from the first A record already in `dns_records`, for
/// targets that have none. Returns rows updated. `project_path=None` = all.
pub async fn backfill_real_ip_from_dns(pool: &PgPool, project_path: Option<&str>) -> Result<u64> {
    let res = sqlx::query(&build_backfill_real_ip_sql())
        .bind(project_path)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}
```
**Verify:** add a unit test asserting `build_backfill_real_ip_sql()` contains
`DISTINCT ON (target_id)`, `record_type = 'A'`, `t.real_ip = ''`. Run
`cd backend && cargo nextest run -p golish-db targets`. Expect pass.
**Commit:** `feat(db): targets real_ip setter + dns backfill helper`.

### Task 0.2 — `land_dns_records` also sets `real_ip`
**File:** `backend/crates/golish-recon-app/src/organization_recon/persistence.rs`
In `land_dns_records`, after collecting `records` for a target, pick the first
`A` (IPv4) value and persist it as `real_ip`. Inside the `while let Some(joined)`
loop, after the upsert loop for a target's records:
```rust
// Primary IP for the host tree: first IPv4 (A) answer; fall back to first AAAA.
if let Some(primary) = records
    .iter()
    .find(|(_, rt, _, _)| *rt == "A")
    .or_else(|| records.first())
{
    let (tid, _, _, ip) = primary;
    let _ = golish_db::repo::targets::set_real_ip_by_id(pool, *tid, ip).await;
}
```
(Place where `target_id` for the group is in scope; `records` is already grouped
per resolved target.)
**Verify:** `cargo nextest run -p golish-recon-app organization_recon` (existing
suite still green). Manual: rerun recon on `Test1`, then
`SELECT count(*) FILTER (WHERE real_ip<>'') FROM targets WHERE target_type='domain'`
> 0.
**Commit:** `feat(recon): land primary real_ip during DNS resolution`.

### Task 0.3 — command: `recon_backfill_real_ip` (one-off + manual)
**File:** recon command module (`targets/cmds.rs` or the recon facade — confirm
the existing `recon_*` command file in 1.x by grep `#[tauri::command]` in
`golish-recon-app`). Add a thin command:
```rust
/// Backfill targets.real_ip from existing dns_records A answers (no new
/// resolution). Returns rows updated. Manual refresh / one-off migration aid.
#[tauri::command]
pub async fn recon_backfill_real_ip(
    state: tauri::State<'_, /* pool holder */>,
    project_path: Option<String>,
) -> Result<u64, String> {
    golish_db::repo::targets::backfill_real_ip_from_dns(pool, project_path.as_deref())
        .await
        .map_err(|e| e.to_string())
}
```
Route via `commands_facade/<domain>.rs` (`pub use`) + register (AGENTS.md M1).
Add the frontend wrapper in `frontend/lib/api/` (M2). Naming
`recon_backfill_real_ip` follows `<domain>_<verb>_<object>` (I4).
**Verify:** `just check` (fmt + lint-rust + typecheck). Invoke once on `Test1`;
confirm `real_ip` populated for the 5 domains that already have A records.
**Commit:** `feat(recon): recon_backfill_real_ip command + FE wrapper`.

---

## Phase 1 — IP-centric tree (frontend, no schema change)

### Task 1.1 — `OrgTreeNode.kind` + `buildHostTree` (TDD)
**File:** `frontend/lib/target-panel/org-tree.ts`
Extend the node so a synthetic IP node is distinguishable from an org:
```ts
export interface OrgTreeNode {
  id: string;
  name: string;
  kind?: "org" | "host" | "bucket"; // default "org"
  children: OrgTreeNode[];
  targets: Target[];
}
```
Add a pure builder. Group each org's targets by `real_ip`; IP-type targets are
their own host node (merging any domains that resolve to that IP value):
```ts
const UNRESOLVED_KEY = "__unresolved__";

export function buildHostTree(
  orgs: Organization[],
  targets: Target[],
  unassignedLabel: string,
  unresolvedLabel: string
): OrgTreeNode[] {
  // 1. org spine (reuse buildOrgTree's org wiring), but attach targets grouped
  //    into host sub-nodes instead of a flat list.
  const base = buildOrgTree(orgs, targets, unassignedLabel);
  const regroup = (node: OrgTreeNode): OrgTreeNode => {
    const flat = node.targets;
    const ipNodes = new Map<string, OrgTreeNode>();
    const ensureHost = (ip: string): OrgTreeNode => {
      let h = ipNodes.get(ip);
      if (!h) { h = { id: `host:${node.id}:${ip}`, name: ip, kind: "host", children: [], targets: [] }; ipNodes.set(ip, h); }
      return h;
    };
    const unresolved: Target[] = [];
    // IP targets seed host nodes keyed by their own value.
    for (const t of flat) if (t.target_type === "ip") ensureHost(t.value).targets.push(t);
    // Domains/URLs attach to the host of their real_ip; else unresolved.
    for (const t of flat) {
      if (t.target_type === "ip") continue;
      const ip = (t.real_ip ?? "").trim();
      if (ip) ensureHost(ip).targets.push(t);
      else unresolved.push(t);
    }
    const hostChildren = [...ipNodes.values()].sort((a, b) => a.name.localeCompare(b.name));
    const buckets: OrgTreeNode[] = [];
    if (unresolved.length) buckets.push({ id: `unresolved:${node.id}`, name: unresolvedLabel, kind: "bucket", children: [], targets: unresolved });
    return { ...node, targets: [], children: [...hostChildren, ...buckets, ...node.children.map(regroup)] };
  };
  return base.map(regroup);
}
```
**File (test):** `frontend/lib/target-panel/host-tree.test.ts`
```ts
import { describe, expect, it } from "vitest";
import { buildHostTree } from "./org-tree";

const org = { id: "o1", name: "Acme", parent_id: null, sort_order: 0 } as any;
const tgt = (over: any) => ({ id: over.value, name: over.value, target_type: "domain", value: over.value, scope: "in", real_ip: "", organization_id: "o1", ...over });

describe("buildHostTree", () => {
  it("nests domains under their real_ip host node", () => {
    const targets = [
      tgt({ value: "1.1.1.1", target_type: "ip" }),
      tgt({ value: "a.com", real_ip: "1.1.1.1" }),
      tgt({ value: "b.com", real_ip: "" }),
    ];
    const roots = buildHostTree([org], targets as any, "Unassigned", "Unresolved");
    const acme = roots[0];
    const host = acme.children.find((c) => c.kind === "host" && c.name === "1.1.1.1")!;
    expect(host.targets.map((t) => t.value)).toContain("a.com");
    const bucket = acme.children.find((c) => c.kind === "bucket")!;
    expect(bucket.targets.map((t) => t.value)).toEqual(["b.com"]);
  });
});
```
**Verify:** `cd frontend && pnpm vitest run lib/target-panel/host-tree.test.ts`.
Expect pass.
**Commit:** `feat(fe): buildHostTree IP-centric grouping + tests`.

### Task 1.2 — render host/bucket nodes in the sidebar
**File:** `frontend/components/TargetPanel/OrgTreeSidebar.tsx`
In `OrgTreeNodeRow`, branch on `node.kind`:
- `"host"` / `"bucket"`: render a simple row (Network icon for host, FolderOpen
  for bucket), the name, `countAllTargets(node).total`, then the assets group +
  recursive children — **omit** the org-action buttons / edit / delete (those
  only make sense for real orgs). Reuse the existing asset sub-group + children
  rendering blocks unchanged.
- default (`"org"` / undefined): current behavior.
Keep `countAllTargets` as-is (it already recurses children + targets).
**Verify:** `just check-fe` (biome + tsc). Manual: toggle on (next task) shows
IP nodes with domains nested, no edit/delete on IP rows.
**Commit:** `feat(fe): OrgTreeSidebar renders host/bucket nodes`.

### Task 1.3 — view toggle in `TargetGroupedView`
**File:** `frontend/components/TargetPanel/TargetGroupedView.tsx`
- Add state: `const [viewMode, setViewMode] = useState<"byType" | "byIp">("byType");`
- Replace the `roots` memo (currently line ~268):
```ts
const unresolvedLabel = t("targets.unresolvedGroup");
const roots = useMemo(
  () =>
    viewMode === "byIp"
      ? buildHostTree(orgs, visibleTargets, unassignedLabel, unresolvedLabel)
      : buildOrgTree(orgs, visibleTargets, unassignedLabel),
  [viewMode, orgs, visibleTargets, unassignedLabel, unresolvedLabel]
);
```
- Add a small segmented toggle in the toolbar (near the existing header actions)
  bound to `viewMode` with labels `t("targets.viewByType")` / `t("targets.viewByIp")`.
- Import `buildHostTree`.
**Verify:** `just check-fe` + manual click-through (toggle flips tree). 
**Commit:** `feat(fe): by-type / by-IP target tree toggle`.

### Task 1.4 — i18n labels
**File:** `frontend/lib/i18n/en.json` + `zh-CN.json`
Add under `targets`: `viewByType` ("By type"/"按类型"), `viewByIp`
("By IP"/"按 IP"), `unresolvedGroup` ("Unresolved"/"未解析域名").
**Verify:** `just check-fe`. **Commit:** `feat(i18n): host-view labels`.

### Task 1.5 — full verification
Run `just precommit`. Capture command + result into `agent-progress.md`
("已记录证据"). Update `feature_list.json` entry → `passing` only if all green.

---

## Phase 2 — host-aware coverage (DEFERRED, documented only)

> **Design now written:** `docs/design/2026-06-15-host-aware-coverage.md`
> (per-asset-type technique matrix; phased 2a/2b/2c; parity-test + flag rollout).
> Implementation plan still pending user sign-off.

Not implemented in this change. When picked up, it touches the harness core
(I7/I8), so do it as its own spec+plan+TDD cycle:
- `golish-db/src/repo/coverage_truth.rs::assemble_truth_facts` — replace the
  uniform 12-technique projection with a **technique↔asset-type matrix**
  (domain-level: DNS/SUBDOMAIN/CT/WHOIS/ASN/OSINT; host-level:
  LIVENESS/PORT/SERVICE-FP; URL-level: DIR/PARAM/JSAPI). The asset axis must
  carry `target_type` (today `in_scope_assets: &[String]`).
- `resources/harness/stages/target_intel.json` — `coverage_complete` expectations
  expressed per asset type (so a domain isn't expected to have a port scan).
- The gate hook that builds `in_scope_assets` + projects facts.
- Heavy unit coverage; verify no gate regression (PASS/BLOCK parity on a known
  run) before/after.
Risk: miscalibrated gate = wrongly PASS (security) or BLOCK (workflow). Requires
explicit sign-off (AGENTS.md §2.7-adjacent: harness-core change).

---

## Self-check (writing-plans)

- Spec coverage: Phase 0 (real_ip persist + backfill) ↔ design §4 Phase 0;
  Phase 1 (tree + toggle) ↔ design §4 Phase 1; Phase 2 deferred ↔ design §4/§2 C.
- Types consistent: `set_real_ip_by_id`, `backfill_real_ip_from_dns`,
  `buildHostTree`, `OrgTreeNode.kind`, `viewMode` used identically across tasks.
- Open confirmations folded into tasks (primary-IP grouping; in-scope-only;
  backfill-once). No "TODO"/placeholder steps.
