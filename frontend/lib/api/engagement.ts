import type { EngagementSnapshot } from "@/lib/generated/EngagementSnapshot";
import type { OrgTreeNode } from "@/lib/generated/OrgTreeNode";
import { invoke } from "./client";

export type { EngagementSnapshot } from "@/lib/generated/EngagementSnapshot";
export type { OrgRunStatusDto } from "@/lib/generated/OrgRunStatusDto";
export type { OrgTreeNode } from "@/lib/generated/OrgTreeNode";
export type { OrgWeaknessScore } from "@/lib/generated/OrgWeaknessScore";

export type EngagementMode = "checklist" | "funnel";

/** Worker scope DTO (mirrors backend `EngagementWorkerScopeDto`, camelCase). */
export interface EngagementWorkerScope {
  orgId: string;
  /** Slice start stage id (e.g. "target_intel"); omit for the DAG's entry. */
  from?: string | null;
  /** Slice end stage id (inclusive), e.g. "enumeration" / "reporting". */
  to: string;
  includeSubsidiaries: boolean;
  subsidiaryThresholdPct: number;
}

/**
 * Pin an engagement worker scope onto a spawned worker session (after
 * init_ai_session + set_execution_mode, BEFORE the seed prompt). The backend
 * Task router then hard-constrains the run to that org + stage slice.
 */
export async function engagementSetWorkerScope(
  sessionId: string,
  scope: EngagementWorkerScope
): Promise<void> {
  return invoke<void>("engagement_set_worker_scope", { sessionId, scope });
}

/** Clear the worker scope (reverts to a normal chat/task session). */
export async function engagementClearWorkerScope(sessionId: string): Promise<void> {
  return invoke<void>("engagement_clear_worker_scope", { sessionId });
}

/** Read back the worker scope pinned on a session (debug / pool resync). */
export async function engagementGetWorkerScope(
  sessionId: string
): Promise<EngagementWorkerScope | null> {
  return invoke<EngagementWorkerScope | null>("engagement_get_worker_scope", { sessionId });
}

/**
 * Read the engagement snapshot (org tree + DB-truth coverage + weakness
 * scores). Read-only: this is the "scope locked" signal after the scoping chat
 * lands the org tree (design 2026-06-13-engagement-scoping-fanout). The
 * worker-pool runtime state (Phase B) overlays on top of this DB-truth base.
 */
export async function getEngagementSnapshot(args?: {
  projectPath?: string;
  mode?: EngagementMode;
  toStage?: string;
}): Promise<EngagementSnapshot> {
  return invoke<EngagementSnapshot>("engagement_get_snapshot", {
    projectPath: args?.projectPath ?? null,
    mode: args?.mode ?? null,
    toStage: args?.toStage ?? null,
  });
}

/** Flatten a forest into [node, depth] pairs in pre-order (for table rendering). */
export function flattenTree(
  roots: OrgTreeNode[],
  expanded: Set<string>,
  depth = 0
): Array<{ node: OrgTreeNode; depth: number }> {
  const out: Array<{ node: OrgTreeNode; depth: number }> = [];
  for (const node of roots) {
    out.push({ node, depth });
    if (expanded.has(node.organizationId) && node.children.length > 0) {
      out.push(...flattenTree(node.children, expanded, depth + 1));
    }
  }
  return out;
}

/**
 * Sort a forest for display. `funnel` ranks by weakness total (desc); `checklist`
 * keeps the backend order (parents-first, name). Recurses into children. Pure —
 * mode only changes ordering, never the data.
 */
export function sortForest(roots: OrgTreeNode[], mode: EngagementMode): OrgTreeNode[] {
  const sortLevel = (nodes: OrgTreeNode[]): OrgTreeNode[] => {
    const copy = nodes.map((n) => ({ ...n, children: sortLevel(n.children) }));
    if (mode === "funnel") {
      copy.sort((a, b) => {
        // ts-rs maps Rust i64 → bigint; coerce to number for arithmetic
        // (weakness scores are small, well within safe-integer range).
        const sa = Number(a.weakness?.total ?? 0);
        const sb = Number(b.weakness?.total ?? 0);
        return sb - sa || a.name.localeCompare(b.name);
      });
    }
    return copy;
  };
  return sortLevel(roots);
}
