import { describe, expect, it } from "vitest";
import type { OrgTreeNode } from "@/lib/target-panel/org-tree";
import { orgTreeNodeHasExpandableContent } from "./OrgTreeSidebar";

const node = (overrides: Partial<OrgTreeNode> = {}): OrgTreeNode => ({
  id: "org-1",
  name: "Org",
  children: [],
  targets: [],
  kind: "org",
  ...overrides,
});

describe("orgTreeNodeHasExpandableContent", () => {
  it("does not show an expand chevron for org leaves with no children", () => {
    expect(orgTreeNodeHasExpandableContent(node(), false)).toBe(false);
  });

  it("shows an expand chevron for orgs with child organizations", () => {
    expect(orgTreeNodeHasExpandableContent(node({ children: [node({ id: "child" })] }), false)).toBe(
      true
    );
  });
});
