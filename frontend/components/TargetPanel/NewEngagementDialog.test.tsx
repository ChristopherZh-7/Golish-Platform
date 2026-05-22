import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { Organization } from "@/lib/api/organizations";
import { NewEngagementDialog } from "./NewEngagementDialog";

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
});
