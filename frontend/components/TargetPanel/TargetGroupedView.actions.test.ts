import { describe, expect, it } from "vitest";

import type {
  AssetIntelProviderDescriptor,
  AssetIntelRun,
  AssetIntelStreamEvent,
} from "@/lib/api/asset-intel";
import {
  applyStreamEvent,
  buildDiscoveryHydrateConfigFromEngagement,
  buildHydrateConfigFromEngagement,
  formatFieldValue,
  getCandidateCounts,
  getCandidateItems,
  getCandidateSourceFilter,
  getEngagementDetails,
  getEvidenceRawRows,
  getNextWorkspaceTabAfterAssetIntelRun,
  getOrgActionModel,
  getOrgFieldGroups,
  getProviderStatusClass,
  getVisibleCandidateBuckets,
  getWorkspaceModel,
  type HydrateActivity,
} from "./TargetGroupedView";

const emptyActivity: HydrateActivity = {
  runId: null,
  providers: {},
  providerOrder: [],
};

describe("getOrgActionModel", () => {
  it("uses different primary actions for each engagement mode", () => {
    expect(getOrgActionModel("customer_targets").primary).toMatchObject({
      kind: "import_targets",
      label: "Import targets",
    });
    expect(getOrgActionModel("discover_assets").primary).toMatchObject({
      kind: "hydrate_subsidiaries",
      label: "查子公司",
    });
    expect(getOrgActionModel("profile_only").primary).toMatchObject({
      kind: "choose_next_step",
      label: "Choose next step",
    });
  });

  it("shows discovery and single-org enrichment actions for a master discovery org", () => {
    const model = getOrgActionModel("discover_assets", { isChild: false });
    const actionKinds = [model.primary.kind, model.secondary?.kind].filter(Boolean);

    expect(model.primary).toMatchObject({
      kind: "hydrate_subsidiaries",
      label: "查子公司",
    });
    expect(model.secondary).toMatchObject({
      kind: "enrich_organization",
      label: "补字段",
    });
    expect(actionKinds).not.toContain("enrich_batch");
  });

  it("shows single-org enrichment for a promoted child discovery org", () => {
    const model = getOrgActionModel("discover_assets", { isChild: true });
    const actionKinds = [model.primary.kind, model.secondary?.kind].filter(Boolean);

    expect(model.primary).toMatchObject({
      kind: "enrich_organization",
      label: "补字段",
    });
    expect(actionKinds).not.toContain("enrich_batch");
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

  it("counts only candidates from allowed provider sources when a filter is provided", () => {
    expect(
      getCandidateCounts(
        {
          candidates: {
            organizations: [
              { id: "org-enscan", source: "enscan-go" },
              { id: "org-zone", source: "0.zone" },
            ],
            targets: [
              { id: "target-zone", source: "0.zone" },
              { id: "target-manual", source: "" },
            ],
          },
        },
        new Set(["enscan-go"])
      )
    ).toEqual({ organizations: 1, targets: 1 });
  });
});

describe("getCandidateItems", () => {
  it("returns typed candidate buckets for review UI", () => {
    const engagement = {
      candidates: {
        organizations: [{ id: "org-a", kind: "organization", label: "Org A", value: "Org A" }],
        targets: [{ id: "target-a", kind: "target", label: "api.example.com", value: "api.example.com" }],
      },
    };

    expect(getCandidateItems(engagement, "organizations")).toHaveLength(1);
    expect(getCandidateItems(engagement, "targets")[0]).toMatchObject({
      id: "target-a",
      kind: "target",
    });
  });

  it("filters stale candidates by the selected asset-intel phase", () => {
    const engagement = {
      candidates: {
        organizations: [
          {
            id: "org:enscan-go:Ping An Bank",
            kind: "organization",
            label: "Ping An Bank",
            value: "Ping An Bank",
            source: "enscan-go",
          },
          {
            id: "org:0.zone:noise",
            kind: "organization",
            label: "0.zone stale org",
            value: "0.zone stale org",
            source: "0.zone",
          },
        ],
        targets: [
          {
            id: "target:0.zone:pa18.com",
            kind: "target",
            label: "pa18.com",
            value: "pa18.com",
            source: "0.zone",
          },
        ],
      },
    };
    const discoverySources = new Set(["enscan-go"]);

    expect(getCandidateItems(engagement, "organizations", discoverySources)).toEqual([
      expect.objectContaining({ source: "enscan-go" }),
    ]);
    expect(getCandidateItems(engagement, "targets", discoverySources)).toEqual([]);
  });
});

describe("getCandidateSourceFilter", () => {
  const providers: AssetIntelProviderDescriptor[] = [
    {
      id: "enscan-go",
      displayName: "ENScan_GO",
      requiresIntegration: null,
      capabilities: ["subsidiaries", "domains"],
      status: "available",
    },
    {
      id: "0.zone",
      displayName: "0.zone（零零信安）",
      requiresIntegration: null,
      capabilities: ["domains", "apps"],
      status: "available",
    },
  ];

  it("separates discovery provider sources from enrichment provider sources", () => {
    expect(getCandidateSourceFilter(providers, "discovery")).toEqual(
      new Set(["enscan-go", "enscan_go"])
    );
    expect(getCandidateSourceFilter(providers, "enrichment")).toEqual(
      new Set(["0.zone", "0.zone（零零信安）"])
    );
  });
});

describe("getVisibleCandidateBuckets", () => {
  it("hides target candidates during the discovery phase", () => {
    const engagement = {
      candidates: {
        organizations: [
          {
            id: "org:enscan-go:Ping An Bank",
            kind: "organization",
            label: "Ping An Bank",
            value: "Ping An Bank",
            source: "enscan-go",
          },
        ],
        targets: [
          {
            id: "target:enscan-go:pingan.com",
            kind: "target",
            label: "pingan.com",
            value: "pingan.com",
            source: "enscan-go",
          },
        ],
      },
    };

    expect(getVisibleCandidateBuckets(engagement, "discovery", new Set(["enscan-go"]))).toEqual({
      organizations: [expect.objectContaining({ source: "enscan-go" })],
      targets: [],
    });
  });
});

describe("buildHydrateConfigFromEngagement", () => {
  it("maps discover-assets engagement settings into hydrate config", () => {
    expect(
      buildHydrateConfigFromEngagement({
        mode: "discover_assets",
        min_ownership_percent: "35",
        depth: "3",
        include_branches: true,
        create_candidates: true,
      })
    ).toEqual({
      minOwnershipPercent: "35",
      depth: "3",
      includeBranches: true,
      createCandidates: true,
    });
  });

  it("treats legacy heavy discovery defaults as a lightweight hydrate config", () => {
    expect(
      buildHydrateConfigFromEngagement({
        mode: "discover_assets",
        min_ownership_percent: "51",
        depth: "2",
        include_branches: true,
        create_candidates: true,
      })
    ).toEqual({
      minOwnershipPercent: null,
      depth: null,
      includeBranches: null,
      createCandidates: true,
    });
  });
});

describe("buildDiscoveryHydrateConfigFromEngagement", () => {
  it("defaults discovery to investment and branch collection instead of target-only hydrate", () => {
    expect(buildDiscoveryHydrateConfigFromEngagement({ mode: "discover_assets" })).toEqual({
      minOwnershipPercent: "51",
      depth: "1",
      includeBranches: false,
      createCandidates: true,
    });
  });

  it("preserves explicit discovery thresholds from the engagement", () => {
    expect(
      buildDiscoveryHydrateConfigFromEngagement({
        mode: "discover_assets",
        min_ownership_percent: "35",
        depth: "2",
        include_branches: false,
        create_candidates: false,
      })
    ).toEqual({
      minOwnershipPercent: "35",
      depth: "2",
      includeBranches: false,
      createCandidates: false,
    });
  });
});

describe("getProviderStatusClass", () => {
  it("maps provider status into visible UI tones", () => {
    expect(getProviderStatusClass("completed")).toContain("green");
    expect(getProviderStatusClass("checked_empty")).toContain("blue");
    expect(getProviderStatusClass("failed")).toContain("red");
    expect(getProviderStatusClass("unavailable")).toContain("red");
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
      intel: {
        records: [],
        mobile_apps: ["小米实况麻将"],
        mini_programs: ["小米商城"],
        app_domains: ["https://com.dfwe"],
        exposed_emails: ["alice@example.com"],
        email_leakage_total: "2",
        code_leaks: ["https://github.com/acme/leak.txt"],
        mail_mx: ["mx1.example.com"],
      },
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
      "Apps & Mini Programs",
      "Domains",
      "Network",
      "Scope",
      "Identity",
      "Surfaces",
      "Leakage Intel",
      "DNS",
      "Risk & Notes",
    ]);

    const leakage = groups.find((group) => group.title === "Leakage Intel");
    expect(leakage?.fields.map((field) => field.key)).toEqual([
      "exposed_emails",
      "email_leakage_total",
      "code_leaks",
    ]);
    expect(leakage?.fields.every((field) => field.filled)).toBe(true);

    const dns = groups.find((group) => group.title === "DNS");
    expect(dns?.fields.map((field) => field.key)).toEqual(["mail_mx"]);
    expect(dns?.fields[0]?.filled).toBe(true);
    expect(groups[0].fields.map((field) => field.key)).toEqual([
      "aliases",
      "industry",
      "tier",
      "credit_code",
    ]);
    expect(groups[3].fields.map((field) => field.key)).toEqual([
      "ip_ranges",
      "asns",
      "email_domains",
    ]);

    const identity = groups.find((group) => group.title === "Identity");
    expect(identity?.fields.map((field) => field.key)).toEqual(["intel", "subsidiaries"]);

    const surfaces = groups.find((group) => group.title === "Surfaces");
    expect(surfaces?.fields.map((field) => field.key)).toEqual([
      "business_systems",
      "cloud_assets",
      "github_orgs",
      "social_accounts",
    ]);

    const apps = groups.find((group) => group.title === "Apps & Mini Programs");
    expect(apps?.fields.map((field) => field.key)).toEqual([
      "mobile_apps",
      "mini_programs",
      "app_domains",
    ]);
    expect(apps?.fields.every((field) => field.filled)).toBe(true);

    const risk = groups.find((group) => group.title === "Risk & Notes");
    expect(risk?.fields.map((field) => field.key)).toEqual([
      "certificates",
      "historical_vulns",
      "contacts",
      "notes",
    ]);
  });

  it("carries the raw value through OrgFieldView so the UI can render chips / records", () => {
    const groups = getOrgFieldGroups({
      domains: [{ domain: "example.com" }, { domain: "api.example.com" }],
      ip_ranges: ["10.0.0.1", "10.0.0.2"],
      intel: { legal_representative: "马明哲", registered_capital: "100万" },
    });
    const domains = groups
      .find((group) => group.title === "Domains")
      ?.fields.find((field) => field.key === "domains");
    expect(domains?.raw).toEqual([{ domain: "example.com" }, { domain: "api.example.com" }]);

    const ip = groups
      .find((group) => group.title === "Network")
      ?.fields.find((field) => field.key === "ip_ranges");
    expect(ip?.raw).toEqual(["10.0.0.1", "10.0.0.2"]);

    const intel = groups
      .find((group) => group.title === "Identity")
      ?.fields.find((field) => field.key === "intel");
    expect(intel?.raw).toEqual({ legal_representative: "马明哲", registered_capital: "100万" });
  });

  it("shows asset-intel keys inside intel instead of a generic configured marker", () => {
    const groups = getOrgFieldGroups({
      intel: {
        engagement: { mode: "discover_assets" },
        legal_representative: "马明哲",
        registered_capital: "1,810,764.1995万(元)",
        business_status: "开业",
      },
    });
    const intel = groups
      .find((group) => group.title === "Identity")
      ?.fields.find((field) => field.key === "intel");

    expect(intel?.value).toContain("Legal: 马明哲");
    expect(intel?.value).toContain("Capital: 1,810,764.1995万(元)");
    expect(intel?.value).not.toBe("configured");
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

describe("getNextWorkspaceTabAfterAssetIntelRun", () => {
  const run = (overrides: Partial<AssetIntelRun> = {}): AssetIntelRun => ({
    runId: "run-1",
    status: "completed",
    providerStatus: [],
    candidates: { organizations: [], targets: [] },
    evidence: [],
    ...overrides,
  });

  it("keeps discovery runs on activity when there are no reviewable candidates", () => {
    expect(getNextWorkspaceTabAfterAssetIntelRun("hydrate_subsidiaries", run())).toBe("activity");
  });

  it("opens candidates only when discovery leaves candidates to review", () => {
    expect(
      getNextWorkspaceTabAfterAssetIntelRun(
        "hydrate_subsidiaries",
        run({
          candidates: {
            organizations: [
              {
                kind: "organization",
                label: "子公司 A",
                value: "子公司 A",
                source: "enscan-go",
              },
            ],
            targets: [],
          },
        })
      )
    ).toBe("candidates");
  });

  it("keeps partial or failed discovery runs on activity even if a provider emitted candidates", () => {
    const candidates = {
      organizations: [
        {
          kind: "organization" as const,
          label: "半截输出子公司",
          value: "半截输出子公司",
          source: "enscan-go-tyc-discovery",
        },
      ],
      targets: [],
    };

    expect(
      getNextWorkspaceTabAfterAssetIntelRun(
        "hydrate_subsidiaries",
        run({ status: "partial", candidates })
      )
    ).toBe("activity");
    expect(
      getNextWorkspaceTabAfterAssetIntelRun(
        "hydrate_subsidiaries",
        run({ status: "failed", candidates })
      )
    ).toBe("activity");
  });

  it("returns null after enrich_organization so the user stays on whichever tab they ran from", () => {
    expect(getNextWorkspaceTabAfterAssetIntelRun("enrich_organization", run())).toBeNull();
    expect(
      getNextWorkspaceTabAfterAssetIntelRun(
        "enrich_organization",
        run({ status: "failed" })
      )
    ).toBeNull();
  });
});

describe("getEvidenceRawRows", () => {
  it("extracts known ENScan fields from candidate.evidence.raw", () => {
    const rows = getEvidenceRawRows({
      provider: "enscan-go",
      runId: "run-1",
      raw: {
        name: "小米科技投资有限公司",
        reg_code: "91110108551385082Q",
        scale: "100",
        legal: "雷军",
        addr: "北京市海淀区",
        industry: "互联网",
        phone: "010-12345678",
        unrelated_garbage: "should-not-show",
      },
    });
    const labels = rows.map((row) => row.label);
    expect(labels).toEqual([
      "Name",
      "Credit code",
      "Ownership %",
      "Legal representative",
      "Industry",
      "Address",
      "Phone",
    ]);
    expect(rows.find((row) => row.label === "Address")?.value).toBe("北京市海淀区");
    expect(rows.some((row) => row.value === "should-not-show")).toBe(false);
  });

  it("ignores nulls / empty strings / object values", () => {
    const rows = getEvidenceRawRows({
      raw: {
        name: "  ",
        legal: null,
        reg_code: undefined,
        addr: { nested: "ignored" },
        scale: 51,
      },
    });
    expect(rows).toHaveLength(1);
    expect(rows[0]).toEqual({ field: "scale", label: "Ownership %", value: "51" });
  });

  it("returns empty array when evidence is missing or wrong shape", () => {
    expect(getEvidenceRawRows(undefined)).toEqual([]);
    expect(getEvidenceRawRows(null)).toEqual([]);
    expect(getEvidenceRawRows({})).toEqual([]);
    expect(getEvidenceRawRows({ raw: [] })).toEqual([]);
    expect(getEvidenceRawRows({ raw: "string-not-object" })).toEqual([]);
  });

  it("dedupes labels when multiple source fields map to the same display label", () => {
    const rows = getEvidenceRawRows({
      raw: {
        reg_code: "AAA",
        credit_code: "BBB",
        addr: "addr-1",
        address: "addr-2",
      },
    });
    const labels = rows.map((row) => row.label);
    expect(labels.filter((label) => label === "Credit code")).toHaveLength(1);
    expect(labels.filter((label) => label === "Address")).toHaveLength(1);
    expect(rows.find((row) => row.label === "Credit code")?.value).toBe("AAA");
  });
});

describe("applyStreamEvent", () => {
  it("creates a provider entry on provider_started and records runId", () => {
    const event: AssetIntelStreamEvent = {
      kind: "provider_started",
      runId: "run-1",
      providerId: "enscan-go",
      displayName: "ENScan_GO",
      runtime: "cli_json",
    };
    const next = applyStreamEvent(emptyActivity, event);
    expect(next.runId).toBe("run-1");
    expect(next.providerOrder).toEqual(["enscan-go"]);
    expect(next.providers["enscan-go"]).toMatchObject({
      displayName: "ENScan_GO",
      runtime: "cli_json",
      state: "running",
      candidateCount: 0,
      batchCount: 0,
    });
  });

  it("appends progress messages keeping only the most recent ones", () => {
    const started = applyStreamEvent(emptyActivity, {
      kind: "provider_started",
      runId: "run-1",
      providerId: "enscan-go",
      displayName: "ENScan_GO",
      runtime: "cli_json",
    });
    let activity = started;
    for (let i = 0; i < 12; i += 1) {
      activity = applyStreamEvent(activity, {
        kind: "provider_progress",
        runId: "run-1",
        providerId: "enscan-go",
        message: `line-${i}`,
        stream: "stdout",
      });
    }
    const provider = activity.providers["enscan-go"];
    expect(provider.recentMessages).toHaveLength(8);
    expect(provider.recentMessages[0]).toBe("line-4");
    expect(provider.recentMessages[7]).toBe("line-11");
  });

  it("accumulates batch counts and candidate counts as deltas arrive", () => {
    const started = applyStreamEvent(emptyActivity, {
      kind: "provider_started",
      runId: "run-1",
      providerId: "enscan-go",
      displayName: "ENScan_GO",
      runtime: "cli_json",
    });
    const afterFirst = applyStreamEvent(started, {
      kind: "provider_batch",
      runId: "run-1",
      providerId: "enscan-go",
      source: "artifact",
      candidates: {
        organizations: [{ id: "org:enscan-go:Acme" } as never],
        targets: [{ id: "target:enscan-go:a.example" } as never],
      },
    });
    const afterSecond = applyStreamEvent(afterFirst, {
      kind: "provider_batch",
      runId: "run-1",
      providerId: "enscan-go",
      source: "artifact",
      candidates: { organizations: [], targets: [{ id: "target:enscan-go:b.example" } as never] },
    });
    expect(afterSecond.providers["enscan-go"]).toMatchObject({
      batchCount: 2,
      candidateCount: 3,
      state: "running",
    });
  });

  it("marks a provider as completed and trusts the final candidate count", () => {
    const started = applyStreamEvent(emptyActivity, {
      kind: "provider_started",
      runId: "run-1",
      providerId: "enscan-go",
      displayName: "ENScan_GO",
      runtime: "cli_json",
    });
    const next = applyStreamEvent(started, {
      kind: "provider_completed",
      runId: "run-1",
      providerId: "enscan-go",
      status: {
        providerId: "enscan-go",
        status: "completed",
        message: "enscan-go normalized 5 candidate(s)",
      },
      candidateCount: 5,
    });
    expect(next.providers["enscan-go"]).toMatchObject({
      state: "completed",
      candidateCount: 5,
      status: { status: "completed" },
    });
  });

  it("ignores progress / batch / completed events for unknown providers", () => {
    const next = applyStreamEvent(emptyActivity, {
      kind: "provider_progress",
      runId: "run-1",
      providerId: "ghost",
      message: "noise",
      stream: "stdout",
    });
    expect(next).toBe(emptyActivity);
  });
});
