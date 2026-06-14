import { describe, expect, it } from "vitest";
import type { Organization } from "@/lib/api/organizations";
import type { Target } from "@/lib/pentest/types";
import { buildOrgTree, countOrgDeletionImpact, UNASSIGNED_KEY } from "./org-tree";

function org(id: string, parentId: string | null = null): Organization {
  return { id, parent_id: parentId, name: id, sort_order: 0 } as Organization;
}

function tgt(id: string, organizationId: string | null): Target {
  return { id, organization_id: organizationId } as Target;
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
