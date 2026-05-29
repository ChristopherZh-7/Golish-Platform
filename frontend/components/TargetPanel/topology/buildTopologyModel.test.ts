import { describe, expect, it } from "vitest";

import type { Organization } from "@/lib/api/organizations";
import type { Target } from "@/lib/pentest/types";
import { applyTopologyFocus, buildTopologyModel, collectLineageIds } from "./buildTopologyModel";

const visible = {
  organization: true,
  target: true,
  service: true,
  evidence: true,
};

function makeOrg(id: string, name: string, parentId: string | null = null): Organization {
  return {
    id,
    project_path: "/tmp/golish",
    name,
    parent_id: parentId,
    description: "",
    owner: "",
    sort_order: 0,
    aliases: [],
    industry: "",
    tier: "",
    credit_code: "",
    domains: [],
    ip_ranges: [],
    asns: [],
    email_domains: [],
    scope_rules: { in: [], out: [] },
    intel: {},
    notes: "",
    certificates: [],
    subsidiaries: [],
    business_systems: [],
    cloud_assets: [],
    github_orgs: [],
    social_accounts: [],
    historical_vulns: [],
    contacts: [],
    created_at: 0,
    updated_at: 0,
  };
}

function makeTarget(id: string, value: string, organizationId: string | null): Target {
  return {
    id,
    name: value,
    type: "domain",
    value,
    tags: [],
    notes: "",
    scope: "in",
    status: "new",
    grp: "",
    owner: "",
    time_window_start: null,
    time_window_end: null,
    organization_id: organizationId,
    source: "customer_provided",
    parent_id: null,
    ports: [],
    technologies: [],
    real_ip: "",
    cdn_waf: "",
    http_title: "",
    http_status: null,
    webserver: "",
    os_info: "",
    content_type: "",
    created_at: 0,
    updated_at: 0,
  };
}

describe("buildTopologyModel", () => {
  it("keeps root-owned and unassigned targets in the target column when sub orgs exist", () => {
    const root = makeOrg("root", "Root");
    const sub = makeOrg("sub", "Sub org", root.id);
    const model = buildTopologyModel(
      [root, sub],
      [
        makeTarget("root-target", "root.example.com", root.id),
        makeTarget("sub-target", "sub.example.com", sub.id),
        makeTarget("unassigned-target", "loose.example.com", null),
      ],
      { mode: "ownership", visibility: visible }
    );

    expect(model.nodes.find((node) => node.id === "target:root-target")).toMatchObject({
      column: 2,
    });
    expect(model.nodes.find((node) => node.id === "target:sub-target")).toMatchObject({
      column: 2,
    });
    expect(model.nodes.find((node) => node.id === "target:unassigned-target")).toMatchObject({
      column: 2,
    });
    expect(model.edges).toContainEqual(
      expect.objectContaining({
        source: root.id,
        target: "target:root-target",
        kind: "contains",
      })
    );
    expect(model.edges).not.toContainEqual(
      expect.objectContaining({
        source: sub.id,
        target: "target:root-target",
      })
    );
  });
});

describe("topology focus / lineage", () => {
  const root = makeOrg("root", "Root");
  const sub = makeOrg("sub", "Sub org", root.id);
  const root2 = makeOrg("root2", "Root Two");

  function model() {
    return buildTopologyModel(
      [root, sub, root2],
      [
        makeTarget("t1", "t1.example.com", root.id),
        makeTarget("t2", "t2.example.com", sub.id),
        makeTarget("t4", "t4.example.com", root2.id),
      ],
      { mode: "ownership", visibility: visible }
    );
  }

  it("collectLineageIds returns the ancestor chain and excludes siblings/cousins", () => {
    const ids = collectLineageIds(model().edges, "target:t2");
    expect([...ids].sort()).toEqual(["root", "sub", "target:t2"]);
    expect(ids.has("target:t1")).toBe(false);
    expect(ids.has("root2")).toBe(false);
  });

  it("isolates a target to its org ancestor chain and hides other branches", () => {
    const focused = applyTopologyFocus(model(), "target:t2");
    const ids = focused.nodes.map((node) => node.id);
    expect(ids).toEqual(expect.arrayContaining(["root", "sub", "target:t2"]));
    expect(ids).not.toContain("target:t1");
    expect(ids).not.toContain("root2");
    expect(ids).not.toContain("target:t4");
    expect(focused.edges.every((edge) => ids.includes(edge.source) && ids.includes(edge.target))).toBe(true);
    expect(Math.min(...focused.nodes.map((node) => node.y))).toBe(86);
  });

  it("isolates an org to its whole descendant subtree and hides sibling roots", () => {
    const focused = applyTopologyFocus(model(), "root");
    const ids = focused.nodes.map((node) => node.id);
    expect(ids).toEqual(expect.arrayContaining(["root", "sub", "target:t1", "target:t2"]));
    expect(ids).not.toContain("root2");
    expect(ids).not.toContain("target:t4");
  });

  it("returns the same model when the focus node is absent or null", () => {
    const base = model();
    expect(applyTopologyFocus(base, "target:does-not-exist")).toBe(base);
    expect(applyTopologyFocus(base, null)).toBe(base);
  });
});
