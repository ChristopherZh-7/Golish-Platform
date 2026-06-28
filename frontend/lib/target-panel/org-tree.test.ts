import { describe, expect, it } from "vitest";
import type { Organization } from "@/lib/api/organizations";
import type { Target } from "@/lib/pentest/types";
import {
  buildHostTree,
  buildOrgTree,
  collectSubtreeTargets,
  countOrgDeletionImpact,
  summarizeTargetCounts,
  UNASSIGNED_KEY,
} from "./org-tree";

function org(id: string, parentId: string | null = null): Organization {
  return { id, parent_id: parentId, name: id, sort_order: 0 } as Organization;
}

function tgt(id: string, organizationId: string | null, scope: "in" | "out" = "in"): Target {
  return { id, organization_id: organizationId, scope } as Target;
}

// P ─ C1 ─ G1   (a 3-level branch)  +  U (unrelated root)
const orgs: Organization[] = [
  org("P"),
  org("C1", "P"),
  org("G1", "C1"),
  org("U"),
];
const targets: Target[] = [
  tgt("t1", "P"),
  tgt("t2", "C1"),
  tgt("t3", "G1"),
  tgt("t4", "U"),
  tgt("t5", null),
];

describe("countOrgDeletionImpact", () => {
  it("counts the whole subtree when deleting a parent org", () => {
    // Deleting P cascades C1 + G1 and every target under the branch.
    expect(countOrgDeletionImpact(orgs, targets, "P")).toEqual({
      subOrgCount: 2,
      targetCount: 3,
    });
  });

  it("counts the child subtree when deleting a mid-level child", () => {
    // Deleting C1 cascades G1 and the targets of C1 + G1, not P's.
    expect(countOrgDeletionImpact(orgs, targets, "C1")).toEqual({
      subOrgCount: 1,
      targetCount: 2,
    });
  });

  it("counts only the leaf's own targets", () => {
    expect(countOrgDeletionImpact(orgs, targets, "G1")).toEqual({
      subOrgCount: 0,
      targetCount: 1,
    });
  });

  it("ignores unassigned targets and targets in other subtrees", () => {
    expect(countOrgDeletionImpact(orgs, targets, "U")).toEqual({
      subOrgCount: 0,
      targetCount: 1,
    });
  });

  it("returns zero for an org with no children or targets", () => {
    expect(countOrgDeletionImpact([org("X")], [], "X")).toEqual({
      subOrgCount: 0,
      targetCount: 0,
    });
  });
});

describe("buildOrgTree unassigned bucket", () => {
  it("buckets targets whose org is missing/deleted as unassigned", () => {
    const roots = buildOrgTree(
      [org("P")],
      [tgt("t1", "P"), tgt("t2", "GONE"), tgt("t3", null)],
      "未分组"
    );
    const unassigned = roots.find((n) => n.id === UNASSIGNED_KEY);
    expect(unassigned?.targets.map((t) => t.id)).toEqual(["t2", "t3"]);
    const p = roots.find((n) => n.id === "P");
    expect(p?.targets.map((t) => t.id)).toEqual(["t1"]);
  });
});

describe("summarizeTargetCounts", () => {
  it("keeps an org's own target count separate from the subtree rollup", () => {
    const roots = buildOrgTree(orgs, [...targets, tgt("t6", "G1", "out")], "未分组");
    const p = roots.find((n) => n.id === "P");
    expect(p).toBeDefined();

    expect(summarizeTargetCounts(p as NonNullable<typeof p>)).toEqual({
      ownTotal: 1,
      ownInScope: 1,
      subtreeTotal: 4,
      subtreeInScope: 3,
      descendantOrgCount: 2,
    });
  });
});

function hostTgt(id: string, patch: Partial<Target>): Target {
  return { id, organization_id: null, type: "domain", value: id, real_ip: "", ...patch } as Target;
}

describe("collectSubtreeTargets", () => {
  it("returns a leaf node's own targets", () => {
    const node = {
      id: "n",
      name: "n",
      children: [],
      targets: [tgt("a", null), tgt("b", null)],
    };
    expect(collectSubtreeTargets(node).map((t) => t.id)).toEqual(["a", "b"]);
  });

  it("recurses through synthetic host + unresolved children of the unassigned group", () => {
    // buildHostTree empties the unassigned node and pushes its orphan targets
    // into host (IP) and unresolved (no real_ip) children. Deleting "未分组"
    // must still reach every underlying row, so the collector has to recurse.
    const roots = buildHostTree(
      [],
      [
        hostTgt("ip1", { type: "ip", value: "1.2.3.4" }),
        hostTgt("d1", { value: "a.com", real_ip: "" }),
        hostTgt("d2", { value: "b.com", real_ip: "1.2.3.4" }),
      ],
      "未分组",
      "未解析域名"
    );
    const unassigned = roots.find((n) => n.id === UNASSIGNED_KEY);
    expect(unassigned).toBeDefined();
    expect(unassigned?.targets).toEqual([]);
    expect(
      collectSubtreeTargets(unassigned as NonNullable<typeof unassigned>)
        .map((t) => t.id)
        .sort()
    ).toEqual(["d1", "d2", "ip1"]);
  });
});
