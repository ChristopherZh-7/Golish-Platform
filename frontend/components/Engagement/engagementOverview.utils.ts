/**
 * Pure helpers behind {@link EngagementOverview} (Phase C, 设计 2026-06-13
 * §10 运行时态 vs DB 真值): merge the live worker-pool state onto the DB-truth
 * snapshot so the overview shows running/queued workers on top of the
 * persisted coverage baseline, and survives reloads by degrading gracefully
 * back to DB truth alone.
 */

import type { OrgRunStatusDto, OrgTreeNode } from "@/lib/api/engagement";
import type { WorkerOutcome, WorkerUnit } from "@/lib/engagement/pool";
import type { RunningWorker } from "@/store/slices/engagement-pool";

/** Pool-aware display status (superset of the DB snapshot's status). */
export type EffectiveStatus = OrgRunStatusDto | "running" | "queued";

export interface PoolView {
  running: Record<string, RunningWorker>;
  queue: WorkerUnit[];
  outcomes: Record<string, WorkerOutcome>;
}

export interface OverviewRow {
  node: OrgTreeNode;
  depth: number;
  effective: EffectiveStatus;
  /** Worker conversation to drill into (most relevant active one). */
  workerConvId: string | null;
}

/** The two unit ids that can affect one org's row. */
function unitIdsForOrg(node: OrgTreeNode): string[] {
  const ids = [`attack:${node.organizationId}`];
  if (node.parentId == null) ids.unshift(`recon:${node.organizationId}`);
  return ids;
}

/** Map a worker outcome onto the display status vocabulary. */
function outcomeToStatus(outcome: WorkerOutcome): EffectiveStatus {
  switch (outcome.status) {
    case "passed":
      return "passed";
    case "skipped":
      return "skippedAlreadyComplete";
    case "blocked":
      return "blocked";
    case "failed":
      return "failed";
  }
}

/**
 * Effective status for one org row: a live worker wins over a queued one,
 * which wins over recorded outcomes (attack outcome preferred over recon —
 * it's the later phase), which win over the DB snapshot status.
 */
export function effectiveStatusFor(node: OrgTreeNode, pool: PoolView): EffectiveStatus {
  const ids = unitIdsForOrg(node);
  for (const id of ids) {
    if (pool.running[id]) return "running";
  }
  if (pool.queue.some((u) => ids.includes(u.id))) return "queued";
  // Attack outcome (ids[last]) describes the org's furthest progress; check it
  // before the family recon outcome.
  for (const id of [...ids].reverse()) {
    const outcome = pool.outcomes[id];
    if (outcome) return outcomeToStatus(outcome);
  }
  return node.status;
}

/** Conversation id of the most relevant worker for this row (running first). */
export function workerConvFor(
  node: OrgTreeNode,
  pool: PoolView,
  findConvByUnit: (unitId: string) => string | null
): string | null {
  const ids = unitIdsForOrg(node);
  for (const id of ids) {
    const running = pool.running[id];
    if (running) return running.convId;
  }
  for (const id of [...ids].reverse()) {
    const conv = findConvByUnit(id);
    if (conv) return conv;
  }
  return null;
}

/**
 * Flatten the org forest into display rows (pre-order, honouring `expanded`)
 * with pool-merged effective statuses.
 */
export function buildOverviewRows(
  tree: OrgTreeNode[],
  expanded: Set<string>,
  pool: PoolView,
  findConvByUnit: (unitId: string) => string | null
): OverviewRow[] {
  const out: OverviewRow[] = [];
  const walk = (nodes: OrgTreeNode[], depth: number) => {
    for (const node of nodes) {
      out.push({
        node,
        depth,
        effective: effectiveStatusFor(node, pool),
        workerConvId: workerConvFor(node, pool, findConvByUnit),
      });
      if (expanded.has(node.organizationId) && node.children.length > 0) {
        walk(node.children, depth + 1);
      }
    }
  };
  walk(tree, 0);
  return out;
}

export interface OverviewSummary {
  totalOrgs: number;
  covered: number;
  active: number;
  queued: number;
  blocked: number;
  failed: number;
}

/** Aggregate counters for the overview header (pool-merged). */
export function summarizeRows(allRows: OverviewRow[]): OverviewSummary {
  const s: OverviewSummary = {
    totalOrgs: allRows.length,
    covered: 0,
    active: 0,
    queued: 0,
    blocked: 0,
    failed: 0,
  };
  for (const r of allRows) {
    switch (r.effective) {
      case "passed":
      case "skippedAlreadyComplete":
        s.covered += 1;
        break;
      case "running":
        s.active += 1;
        break;
      case "queued":
        s.queued += 1;
        break;
      case "blocked":
        s.blocked += 1;
        break;
      case "failed":
        s.failed += 1;
        break;
      default:
        break;
    }
  }
  return s;
}
