/**
 * Org/target tree model for the Target panel.
 *
 * Pure data transforms extracted from `TargetGroupedView.tsx`: build the
 * redteam tree (orgs form the spine via `parent_id`, targets attach to their
 * `organization_id`, orphans land in an "unassigned" bucket) and count
 * targets recursively.
 */

import type { Organization } from "@/lib/api/organizations";
import type { Target } from "@/lib/pentest/types";

export const UNASSIGNED_KEY = "__unassigned__";
export const ROOT_PARENT_KEY = "__root__";

export interface OrgTreeNode {
  id: string;
  name: string;
  /**
   * Node kind. `"org"` (default/undefined) is a real organization row;
   * `"host"` is a synthetic IP/host group (IP-centric view); `"bucket"` is a
   * synthetic catch-all group (e.g. unresolved domains). Only org nodes carry
   * org-level actions in the sidebar.
   */
  kind?: "org" | "host" | "bucket";
  children: OrgTreeNode[];
  targets: Target[];
}

export interface TargetCountSummary {
  ownTotal: number;
  ownInScope: number;
  subtreeTotal: number;
  subtreeInScope: number;
  descendantOrgCount: number;
}

/**
 * Build the redteam tree: organizations form the spine (parent_id chain),
 * targets attach to their `organization_id`, and any orphan targets land in
 * a dedicated "unassigned" bucket at the bottom so legacy/imported data is
 * still reachable rather than silently hidden.
 */
export function buildOrgTree(
  orgs: Organization[],
  targets: Target[],
  unassignedLabel: string
): OrgTreeNode[] {
  const nodeMap = new Map<string, OrgTreeNode>();
  for (const o of orgs) {
    nodeMap.set(o.id, { id: o.id, name: o.name, children: [], targets: [] });
  }

  const roots: OrgTreeNode[] = [];
  for (const o of orgs) {
    const node = nodeMap.get(o.id);
    if (!node) continue;
    if (o.parent_id && nodeMap.has(o.parent_id)) {
      nodeMap.get(o.parent_id)!.children.push(node);
    } else {
      roots.push(node);
    }
  }

  const unassigned: OrgTreeNode = {
    id: UNASSIGNED_KEY,
    name: unassignedLabel,
    children: [],
    targets: [],
  };
  for (const t of targets) {
    const orgId = t.organization_id;
    if (orgId && nodeMap.has(orgId)) {
      nodeMap.get(orgId)!.targets.push(t);
    } else {
      unassigned.targets.push(t);
    }
  }

  const orderByOrg = new Map<string, number>();
  orgs.forEach((o, idx) => {
    orderByOrg.set(o.id, o.sort_order * 1_000_000 + idx);
  });

  const sortNodes = (nodes: OrgTreeNode[]): void => {
    nodes.sort((a, b) => {
      if (a.id === UNASSIGNED_KEY) return 1;
      if (b.id === UNASSIGNED_KEY) return -1;
      return (
        (orderByOrg.get(a.id) ?? 0) - (orderByOrg.get(b.id) ?? 0) ||
        a.name.localeCompare(b.name, "zh")
      );
    });
    for (const n of nodes) sortNodes(n.children);
  };
  sortNodes(roots);

  if (unassigned.targets.length > 0) {
    roots.push(unassigned);
  }
  return roots;
}

/** Key for the synthetic "unresolved" bucket (domains/URLs with no `real_ip`). */
export const UNRESOLVED_KEY = "__unresolved__";

/**
 * IP/host-centric view (design 2026-06-15 Option B, Phase 1): build the same org
 * spine as {@link buildOrgTree}, but regroup each org's flat target list into
 * synthetic **host** nodes keyed by IP. IP-type targets seed their own host node
 * (by `value`); domains/URLs attach to the host of their resolved `real_ip`.
 * Anything without a resolved IP lands in a per-org "unresolved" **bucket** so
 * nothing is hidden (AGENTS.md I8: unchecked ≠ empty). `dns_records` stays the
 * M:N truth; this tree is a derived projection over `targets.real_ip`.
 */
export function buildHostTree(
  orgs: Organization[],
  targets: Target[],
  unassignedLabel: string,
  unresolvedLabel: string
): OrgTreeNode[] {
  const base = buildOrgTree(orgs, targets, unassignedLabel);

  const regroup = (node: OrgTreeNode): OrgTreeNode => {
    const flat = node.targets;
    const hosts = new Map<string, OrgTreeNode>();
    const ensureHost = (ip: string): OrgTreeNode => {
      let host = hosts.get(ip);
      if (!host) {
        host = { id: `host:${node.id}:${ip}`, name: ip, kind: "host", children: [], targets: [] };
        hosts.set(ip, host);
      }
      return host;
    };

    // IP targets seed host nodes keyed by their own value.
    for (const target of flat) {
      if (target.type === "ip") ensureHost(target.value).targets.push(target);
    }
    // Domains/URLs attach to the host of their resolved IP; else unresolved.
    const unresolved: Target[] = [];
    for (const target of flat) {
      if (target.type === "ip") continue;
      const ip = (target.real_ip ?? "").trim();
      if (ip) ensureHost(ip).targets.push(target);
      else unresolved.push(target);
    }

    const hostChildren = [...hosts.values()].sort((a, b) => a.name.localeCompare(b.name));
    const buckets: OrgTreeNode[] = [];
    if (unresolved.length > 0) {
      buckets.push({
        id: `${UNRESOLVED_KEY}:${node.id}`,
        name: unresolvedLabel,
        kind: "bucket",
        children: [],
        targets: unresolved,
      });
    }

    return {
      ...node,
      targets: [],
      children: [...hostChildren, ...buckets, ...node.children.map(regroup)],
    };
  };

  return base.map(regroup);
}

export function countAllTargets(node: OrgTreeNode): { total: number; inScope: number } {
  const summary = summarizeTargetCounts(node);
  return { total: summary.subtreeTotal, inScope: summary.subtreeInScope };
}

export function summarizeTargetCounts(node: OrgTreeNode): TargetCountSummary {
  const ownTotal = node.targets.length;
  const ownInScope = node.targets.filter((t) => t.scope === "in").length;
  let subtreeTotal = ownTotal;
  let subtreeInScope = ownInScope;
  let descendantOrgCount = 0;

  for (const child of node.children) {
    const sub = summarizeTargetCounts(child);
    subtreeTotal += sub.subtreeTotal;
    subtreeInScope += sub.subtreeInScope;
    if ((child.kind ?? "org") === "org" && child.id !== UNASSIGNED_KEY) {
      descendantOrgCount += 1 + sub.descendantOrgCount;
    } else {
      descendantOrgCount += sub.descendantOrgCount;
    }
  }

  return { ownTotal, ownInScope, subtreeTotal, subtreeInScope, descendantOrgCount };
}

export function findOrgTreeNode(nodes: OrgTreeNode[], id: string): OrgTreeNode | null {
  for (const node of nodes) {
    if (node.id === id) return node;
    const child = findOrgTreeNode(node.children, id);
    if (child) return child;
  }
  return null;
}

/**
 * Flatten every real target under a node — its own `targets` plus all
 * descendants'. Synthetic groups (the "unassigned" bucket, per-org "unresolved"
 * buckets, IP "host" nodes) carry their rows on the node *or* its children
 * (e.g. `buildHostTree` empties the unassigned node and pushes its targets into
 * host/bucket children), so a correct "delete this group's targets" action must
 * recurse exactly like {@link countAllTargets}. Returns the underlying `Target`
 * rows so callers can map to ids for deletion and show an accurate count.
 */
export function collectSubtreeTargets(node: OrgTreeNode): Target[] {
  const out: Target[] = [...node.targets];
  for (const child of node.children) out.push(...collectSubtreeTargets(child));
  return out;
}

/**
 * Blast radius of deleting an organization. Mirrors the DB cascade
 * (`organizations.parent_id` + `targets.organization_id`, both ON DELETE
 * CASCADE): deleting an org drops every descendant org and every target owned
 * by the org or any descendant. Used to warn the user up front in the delete
 * confirm dialog.
 *
 * `subOrgCount` excludes the org itself (it's the count of descendant orgs);
 * `targetCount` is every target attached anywhere in the subtree.
 */
export function countOrgDeletionImpact(
  orgs: Organization[],
  targets: Target[],
  orgId: string
): { subOrgCount: number; targetCount: number } {
  const childrenByParent = new Map<string, string[]>();
  for (const o of orgs) {
    if (!o.parent_id) continue;
    const siblings = childrenByParent.get(o.parent_id);
    if (siblings) siblings.push(o.id);
    else childrenByParent.set(o.parent_id, [o.id]);
  }

  const subtree = new Set<string>();
  const stack = [orgId];
  while (stack.length > 0) {
    const id = stack.pop() as string;
    if (subtree.has(id)) continue;
    subtree.add(id);
    for (const child of childrenByParent.get(id) ?? []) stack.push(child);
  }

  const targetCount = targets.filter(
    (t) => t.organization_id != null && subtree.has(t.organization_id)
  ).length;
  // subtree always contains orgId itself; descendants are the rest.
  return { subOrgCount: Math.max(0, subtree.size - 1), targetCount };
}
