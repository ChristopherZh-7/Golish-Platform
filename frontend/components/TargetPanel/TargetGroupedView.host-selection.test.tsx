import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { Target } from "@/lib/pentest/types";

const mocks = vi.hoisted(() => ({
  listOrganizations: vi.fn(async () => [
    {
      id: "org-1",
      name: "Acme",
      parent_id: null,
      project_path: "/workspace",
    },
  ]),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({ save: vi.fn() }));

vi.mock("@/lib/api", () => ({
  assetIntel: {
    listProviders: vi.fn(async () => []),
    listenStream: vi.fn(async () => vi.fn()),
  },
  organizationRecon: { listenStream: vi.fn(async () => vi.fn()) },
  organizations: { listOrganizations: mocks.listOrganizations },
}));

vi.mock("@/lib/events", () => ({
  onCustomEvent: vi.fn(async () => vi.fn()),
  onEvent: vi.fn(async () => vi.fn()),
  sendCustomEvent: vi.fn(async () => undefined),
}));

vi.mock("@/lib/projects", () => ({ getProjectPath: () => "/workspace" }));
vi.mock("@/lib/run-tauri-unlisten", () => ({ runTauriUnlistenFromPromise: vi.fn() }));
vi.mock("./OrgTreeSidebar", () => ({ OrgTreeSidebar: () => <div>organization tree</div> }));
vi.mock("./TargetSurfaceWorkbench", () => ({
  TargetSurfaceWorkbench: ({
    target,
    relatedDomains,
  }: {
    target: Target;
    relatedDomains: Target[];
  }) => (
    <div>
      surface:{target.value}:domains:{relatedDomains.map((domain) => domain.value).join(",")}
    </div>
  ),
}));
vi.mock("./NewEngagementDialog", () => ({ NewEngagementDialog: () => null }));

import { TargetGroupedView } from "./TargetGroupedView";

function resolutionOnlyDomain(): Target {
  return {
    id: "domain-1",
    name: "xblrgame.youchuang7.com",
    type: "domain",
    value: "xblrgame.youchuang7.com",
    scope: "in",
    status: "new",
    grp: "",
    owner: "",
    tags: [],
    notes: "",
    time_window_start: null,
    time_window_end: null,
    organization_id: "org-1",
    source: "customer_provided",
    parent_id: null,
    real_ip: "180.101.153.106",
    cdn_waf: "",
    http_title: "",
    http_status: null,
    webserver: "",
    os_info: "",
    content_type: "",
    created_at: 0,
    updated_at: 0,
    ports: [],
  };
}

describe("TargetGroupedView host selection", () => {
  it("opens a synthetic IP workbench when only a resolving domain exists", async () => {
    render(
      <TargetGroupedView
        targets={[resolutionOnlyDomain()]}
        t={(key) => key}
        onAdd={vi.fn(async () => null)}
        onBatchAdd={vi.fn(async () => [])}
        onDelete={vi.fn(async () => undefined)}
        onDeleteMany={vi.fn(async () => undefined)}
        onReloadTargets={vi.fn(async () => undefined)}
        onToggleScope={vi.fn(async () => undefined)}
        onUpdateNotes={vi.fn()}
      />
    );

    fireEvent.click(await screen.findByRole("button", { name: "180.101.153.106" }));

    expect(
      await screen.findByText("surface:180.101.153.106:domains:xblrgame.youchuang7.com")
    ).toBeVisible();
  });
});
