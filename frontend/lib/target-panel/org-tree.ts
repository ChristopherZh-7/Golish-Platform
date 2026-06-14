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
  children: OrgTreeNode[];
  targets: Target[];
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

export function countAllTargets(node: OrgTreeNode): { total: number; inScope: number } {
  let total = node.targets.length;
  let inScope = node.targets.filter((t) => t.scope === "in").length;
  for (const child of node.children) {
    const sub = countAllTargets(child);
    total += sub.total;
    inScope += sub.inScope;
  }
  return { total, inScope };
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
