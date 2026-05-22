import { describe, expect, it } from "vitest";

import {
  formatFieldValue,
  getCandidateCounts,
  getEngagementDetails,
  getOrgActionModel,
  getOrgFieldGroups,
  getWorkspaceModel,
} from "./TargetGroupedView";

describe("getOrgActionModel", () => {
  it("uses different primary actions for each engagement mode", () => {
    expect(getOrgActionModel("customer_targets").primary).toMatchObject({
      kind: "import_targets",
      label: "Import targets",
    });
    expect(getOrgActionModel("discover_assets").primary).toMatchObject({
      kind: "hydrate_intel",
      label: "Hydrate intel",
    });
    expect(getOrgActionModel("profile_only").primary).toMatchObject({
      kind: "choose_next_step",
      label: "Choose next step",
    });
  });

  it("falls back to profile-only actions when mode is unknown", () => {
    expect(getOrgActionModel(null).primary.kind).toBe("choose_next_step");
  });
});

describe("getWorkspaceModel", () => {
  it("chooses different default right-side workspaces for each mode", () => {
    expect(getWorkspaceModel("customer_targets")).toMatchObject({
      title: "Targets & Testing",
    });
    expect(getWorkspaceModel("discover_assets")).toMatchObject({
      title: "Scope & Intel",
    });
    expect(getWorkspaceModel("profile_only")).toMatchObject({
      title: "Overview",
    });
  });
});

describe("getEngagementDetails", () => {
  it("extracts discovery settings for the right-side workspace", () => {
    expect(
      getEngagementDetails({
        mode: "discover_assets",
        min_ownership_percent: "35",
        depth: "3",
        include_branches: true,
        create_candidates: true,
      })
    ).toEqual([
      ["Min ownership", "35%"],
      ["Depth", "3"],
      ["Branches", "included"],
      ["Candidates", "review first"],
    ]);
  });

  it("shows customer target source and count when available", () => {
    expect(
      getEngagementDetails({
        mode: "customer_targets",
        source: "customer_provided",
        target_count: 12,
      })
    ).toEqual([
      ["Source", "customer_provided"],
      ["Imported targets", "12"],
    ]);
  });
});

describe("getCandidateCounts", () => {
  it("counts organization and target candidates from engagement metadata", () => {
    expect(
      getCandidateCounts({
        candidates: {
          organizations: [{ id: "org-a" }, { id: "org-b" }],
          targets: [{ id: "target-a" }],
        },
      })
    ).toEqual({ organizations: 2, targets: 1 });
  });
});

describe("getOrgFieldGroups", () => {
  it("maps organization fields into UI groups", () => {
    const groups = getOrgFieldGroups({
      aliases: ["Acme"],
      industry: "Finance",
      tier: "critical",
      credit_code: "911",
      domains: [{ domain: "example.com" }],
      ip_ranges: ["10.0.0.0/8"],
      asns: ["AS123"],
      email_domains: ["example.com"],
      scope_rules: { in: ["example.com"] },
      intel: { records: [] },
      notes: "note",
      certificates: [],
      subsidiaries: [],
      business_systems: [],
      cloud_assets: [],
      github_orgs: [],
      social_accounts: [],
      historical_vulns: [],
      contacts: [],
    });
    expect(groups.map((group) => group.title)).toEqual([
      "Basic",
      "Domains",
      "Network",
      "Scope",
      "Other",
    ]);
    expect(groups[0].fields.map((field) => field.key)).toEqual([
      "aliases",
      "industry",
      "tier",
      "credit_code",
    ]);
    expect(groups[2].fields.map((field) => field.key)).toEqual([
      "ip_ranges",
      "asns",
      "email_domains",
    ]);
  });
});

describe("formatFieldValue", () => {
  it("shows concrete samples for array fields instead of only counts", () => {
    expect(formatFieldValue(["a.com", "b.com", "c.com", "d.com"])).toBe("a.com, b.com, c.com +1");
  });

  it("shows domain object arrays by their domain field", () => {
    expect(formatFieldValue([{ domain: "example.com" }, { domain: "api.example.com" }])).toBe(
      "example.com, api.example.com"
    );
  });
});
