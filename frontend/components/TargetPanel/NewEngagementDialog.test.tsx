import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AssetIntelLookupResult } from "@/lib/api/asset-intel";
import type { Organization } from "@/lib/api/organizations";
import { NewEngagementDialog } from "./NewEngagementDialog";

const lookupCompanyMock = vi.fn<(args: { keyword: string }) => Promise<AssetIntelLookupResult>>();

vi.mock("@/lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/api")>();
  return {
    ...actual,
    assetIntel: {
      ...actual.assetIntel,
      lookupCompany: (args: { keyword: string }) => lookupCompanyMock(args),
    },
  };
});

function makeOrg(overrides: Partial<Organization> = {}): Organization {
  return {
    id: "org-1",
    project_path: "",
    name: "Acme Corp",
    parent_id: null,
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
    scope_rules: {},
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
    ...overrides,
  };
}

describe("NewEngagementDialog", () => {
  beforeEach(() => {
    lookupCompanyMock.mockReset();
  });

  it("creates an organization and imports customer-provided targets under it", async () => {
    const user = userEvent.setup();
    const onCreateOrganization = vi.fn().mockResolvedValue(makeOrg({ id: "org-123" }));
    const onUpdateOrganizationProfile = vi.fn().mockResolvedValue(makeOrg({ id: "org-123" }));
    const onBatchAddTargets = vi.fn().mockResolvedValue([]);
    const onCreated = vi.fn();

    render(
      <NewEngagementDialog
        open={true}
        onOpenChange={vi.fn()}
        onCreateOrganization={onCreateOrganization}
        onUpdateOrganizationProfile={onUpdateOrganizationProfile}
        onBatchAddTargets={onBatchAddTargets}
        onCreated={onCreated}
      />
    );

    await user.type(screen.getByLabelText(/organization name/i), "Acme Corp");
    await user.type(
      screen.getByLabelText(/targets/i),
      "example.com\nhttps://portal.example.com\n\nexample.com"
    );
    await user.click(screen.getByRole("button", { name: /create & import/i }));

    await waitFor(() => expect(onCreateOrganization).toHaveBeenCalledTimes(1));
    expect(onCreateOrganization).toHaveBeenCalledWith({
      name: "Acme Corp",
      owner: "",
      description: "",
    });
    expect(onUpdateOrganizationProfile).toHaveBeenCalledWith(
      "org-123",
      expect.objectContaining({
        intel: expect.objectContaining({
          engagement: expect.objectContaining({ mode: "customer_targets" }),
        }),
      })
    );
    expect(onBatchAddTargets).toHaveBeenCalledTimes(1);
    expect(onBatchAddTargets).toHaveBeenCalledWith(
      "example.com\nhttps://portal.example.com",
      "org-123",
      "customer_provided"
    );
    expect(onCreated).toHaveBeenCalledTimes(1);
  });

  it("hides the look-up affordance outside discover_assets mode", () => {
    render(
      <NewEngagementDialog
        open={true}
        onOpenChange={vi.fn()}
        onCreateOrganization={vi.fn()}
        onUpdateOrganizationProfile={vi.fn()}
        onBatchAddTargets={vi.fn()}
        onCreated={vi.fn()}
      />
    );
    expect(screen.queryByRole("button", { name: /look up company/i })).toBeNull();
  });

  it("runs the canonical-company lookup and writes the chosen credit code into the profile patch", async () => {
    const user = userEvent.setup();
    const onCreateOrganization = vi.fn().mockResolvedValue(makeOrg({ id: "org-555" }));
    const onUpdateOrganizationProfile = vi.fn().mockResolvedValue(makeOrg({ id: "org-555" }));
    lookupCompanyMock.mockResolvedValue({
      runId: "run-1",
      matches: [
        {
          providerId: "enscan-go",
          name: "小米科技有限责任公司",
          creditCode: "91110108551385082Q",
          industry: "互联网",
          legalRepresentative: "雷军",
          address: "北京市海淀区清河中街68号",
          registeredAt: "2010-03-03",
          confidence: 0.68,
          evidence: {},
        },
        {
          providerId: "enscan-go",
          name: "小米通讯技术有限公司",
          creditCode: "91440300325990618B",
          confidence: 0.5,
          evidence: {},
        },
      ],
      providerStatus: [
        { providerId: "enscan-go", status: "completed", message: "2 matches" },
      ],
    } satisfies AssetIntelLookupResult);

    render(
      <NewEngagementDialog
        open={true}
        onOpenChange={vi.fn()}
        onCreateOrganization={onCreateOrganization}
        onUpdateOrganizationProfile={onUpdateOrganizationProfile}
        onBatchAddTargets={vi.fn()}
        onCreated={vi.fn()}
        initialMode="discover_assets"
      />
    );

    await user.type(screen.getByLabelText(/organization name/i), "小米");
    await user.click(screen.getByRole("button", { name: /look up company/i }));

    await waitFor(() => expect(lookupCompanyMock).toHaveBeenCalledWith({ keyword: "小米" }));
    await screen.findByText("小米科技有限责任公司");
    expect(screen.getByText("小米通讯技术有限公司")).toBeInTheDocument();

    await user.click(
      screen.getByRole("button", { name: /小米科技有限责任公司/ })
    );

    expect((screen.getByLabelText(/organization name/i) as HTMLInputElement).value).toBe(
      "小米科技有限责任公司"
    );

    await user.click(screen.getByRole("button", { name: /create & prepare discovery/i }));

    await waitFor(() => expect(onUpdateOrganizationProfile).toHaveBeenCalledTimes(1));
    const patch = onUpdateOrganizationProfile.mock.calls[0][1] as {
      credit_code?: string;
      industry?: string;
      intel?: { engagement?: { lookup_match?: { credit_code?: string | null } } };
    };
    expect(patch.credit_code).toBe("91110108551385082Q");
    expect(patch.industry).toBe("互联网");
    expect(patch.intel?.engagement?.lookup_match?.credit_code).toBe("91110108551385082Q");
  });

  it("surfaces lookup errors inline without picking a match", async () => {
    const user = userEvent.setup();
    lookupCompanyMock.mockResolvedValue({
      runId: "run-empty",
      matches: [],
      providerStatus: [
        { providerId: "enscan-go", status: "checked_empty", message: "'enscan-go' lookup found no matches" },
      ],
    });

    render(
      <NewEngagementDialog
        open={true}
        onOpenChange={vi.fn()}
        onCreateOrganization={vi.fn()}
        onUpdateOrganizationProfile={vi.fn()}
        onBatchAddTargets={vi.fn()}
        onCreated={vi.fn()}
        initialMode="discover_assets"
      />
    );

    await user.type(screen.getByLabelText(/organization name/i), "unknown-co");
    await user.click(screen.getByRole("button", { name: /look up company/i }));

    await screen.findByText(/no matches/i);
  });

  it("defaults discover-assets hydration to a lightweight run", async () => {
    const user = userEvent.setup();
    const onCreateOrganization = vi.fn().mockResolvedValue(makeOrg({ id: "org-123" }));
    const onUpdateOrganizationProfile = vi.fn().mockResolvedValue(makeOrg({ id: "org-123" }));
    const onBatchAddTargets = vi.fn().mockResolvedValue([]);

    render(
      <NewEngagementDialog
        open={true}
        onOpenChange={vi.fn()}
        onCreateOrganization={onCreateOrganization}
        onUpdateOrganizationProfile={onUpdateOrganizationProfile}
        onBatchAddTargets={onBatchAddTargets}
        onCreated={vi.fn()}
        initialMode="discover_assets"
      />
    );

    await user.type(screen.getByLabelText(/organization name/i), "Ping An");
    expect(screen.queryByText(/create target candidates after review/i)).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /create & prepare discovery/i }));

    await waitFor(() => expect(onUpdateOrganizationProfile).toHaveBeenCalledTimes(1));
    expect(onUpdateOrganizationProfile).toHaveBeenCalledWith(
      "org-123",
      expect.objectContaining({
        intel: expect.objectContaining({
          engagement: expect.objectContaining({
            mode: "discover_assets",
            min_ownership_percent: "",
            depth: "",
            include_branches: false,
          }),
        }),
      })
    );
    const profilePatch = onUpdateOrganizationProfile.mock.calls[0]?.[1];
    expect(profilePatch?.intel?.engagement).not.toHaveProperty("create_candidates");
  });
});
