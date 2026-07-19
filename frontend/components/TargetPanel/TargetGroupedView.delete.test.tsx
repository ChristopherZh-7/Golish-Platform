import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api/client";
import en from "@/lib/i18n/en.json";
import zhCN from "@/lib/i18n/zh-CN.json";

const mocks = vi.hoisted(() => ({
  deleteOrganization: vi.fn(),
  listOrganizations: vi.fn(),
  onReloadTargets: vi.fn(),
  organizationPresent: true,
  postDeleteStaleReads: 0,
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  save: vi.fn(),
}));

vi.mock("@/lib/api", () => ({
  assetIntel: {
    listProviders: vi.fn(async () => []),
    listenStream: vi.fn(async () => vi.fn()),
  },
  organizationRecon: {
    listenStream: vi.fn(async () => vi.fn()),
  },
  organizations: {
    deleteOrganization: mocks.deleteOrganization,
    listOrganizations: mocks.listOrganizations,
  },
}));

function organizationList() {
  const organization = {
    id: "org-1",
    name: "Acme",
    parent_id: null,
    project_path: "/workspace",
  };
  if (mocks.organizationPresent) return [organization];
  if (mocks.postDeleteStaleReads > 0) {
    mocks.postDeleteStaleReads -= 1;
    return [organization];
  }
  return [];
}

vi.mock("@/lib/events", () => ({
  onCustomEvent: vi.fn(async () => vi.fn()),
  onEvent: vi.fn(async () => vi.fn()),
  sendCustomEvent: vi.fn(async () => undefined),
}));

vi.mock("@/lib/projects", () => ({
  getProjectPath: () => "/workspace",
}));

vi.mock("@/lib/run-tauri-unlisten", () => ({
  runTauriUnlistenFromPromise: vi.fn(),
}));

vi.mock("./OrgTreeSidebar", () => ({
  OrgTreeSidebar: ({ handleDeleteOrg }: { handleDeleteOrg: (id: string, name: string) => void }) => (
    <button type="button" onClick={() => handleDeleteOrg("org-1", "Acme")}>
      request organization delete
    </button>
  ),
}));

vi.mock("./OrgWorkspacePanel", () => ({
  OrgWorkspacePanel: () => <div>workspace</div>,
}));

vi.mock("./TargetSurfaceWorkbench", () => ({
  TargetSurfaceWorkbench: () => <div>surface</div>,
}));

vi.mock("./NewEngagementDialog", () => ({
  NewEngagementDialog: () => null,
}));

import { TargetGroupedView } from "./TargetGroupedView";

const translations: Record<string, string> = {
  "common.cancel": "Cancel",
  "common.delete": "Delete",
  "organizations.deleteConfirm":
    "Delete {{name}} with {{subOrgCount}} sub-organizations and {{targetCount}} targets?",
};

describe("TargetGroupedView delete confirmation", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.organizationPresent = true;
    mocks.postDeleteStaleReads = 1;
    mocks.listOrganizations.mockImplementation(async () => organizationList());
    mocks.deleteOrganization.mockImplementation(async () => {
      mocks.organizationPresent = false;
    });
    mocks.onReloadTargets.mockResolvedValue(undefined);
    Object.defineProperty(window, "confirm", {
      configurable: true,
      value: vi.fn(() => {
        throw new Error("native confirm must not be used");
      }),
    });
  });

  it("discloses that only paused stage tasks without active authority are stopped", () => {
    expect(en.organizations.deleteConfirm).toContain(
      "Paused stage tasks without an active executor will be stopped"
    );
    expect(zhCN.organizations.deleteConfirm).toContain(
      "没有活动执行者的已暂停阶段任务会被停止"
    );
  });

  it("requires the in-app confirmation before deleting an organization", async () => {
    render(
      <TargetGroupedView
        targets={[]}
        t={(key) => translations[key] ?? key}
        onAdd={vi.fn(async () => null)}
        onBatchAdd={vi.fn(async () => [])}
        onDelete={vi.fn(async () => undefined)}
        onDeleteMany={vi.fn(async () => undefined)}
        onReloadTargets={mocks.onReloadTargets}
        onToggleScope={vi.fn(async () => undefined)}
        onUpdateNotes={vi.fn()}
      />
    );

    const requestDelete = await screen.findByRole("button", {
      name: "request organization delete",
    });
    fireEvent.click(requestDelete);

    expect(window.confirm).not.toHaveBeenCalled();
    expect(screen.getByText("Delete Acme with 0 sub-organizations and 0 targets?")).toBeVisible();
    expect(mocks.deleteOrganization).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(mocks.deleteOrganization).not.toHaveBeenCalled();

    fireEvent.click(requestDelete);
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));

    await waitFor(() => expect(mocks.listOrganizations).toHaveBeenCalledTimes(2));
    expect(mocks.onReloadTargets).not.toHaveBeenCalled();
    expect(screen.getByText("Delete Acme with 0 sub-organizations and 0 targets?")).toBeVisible();

    await waitFor(() =>
      expect(mocks.deleteOrganization).toHaveBeenCalledWith({
        id: "org-1",
        projectPath: "/workspace",
      })
    );
    await waitFor(() => expect(mocks.onReloadTargets).toHaveBeenCalledOnce());
  });

  it("shows an actionable active-stage blocker without polling deletion", async () => {
    mocks.deleteOrganization.mockRejectedValueOnce(
      new ApiError(
        "organization_delete",
        {
          code: "ORGANIZATION_DELETE_ACTIVE_STAGE_FORK",
          message:
            "Organization deletion is blocked by active stage fork operation-1 (vuln_triage, task waiting)",
        },
        "deadbeef"
      )
    );
    render(
      <TargetGroupedView
        targets={[]}
        t={(key) => translations[key] ?? key}
        onAdd={vi.fn(async () => null)}
        onBatchAdd={vi.fn(async () => [])}
        onDelete={vi.fn(async () => undefined)}
        onDeleteMany={vi.fn(async () => undefined)}
        onReloadTargets={mocks.onReloadTargets}
        onToggleScope={vi.fn(async () => undefined)}
        onUpdateNotes={vi.fn()}
      />
    );

    fireEvent.click(
      await screen.findByRole("button", {
        name: "request organization delete",
      })
    );
    const readsBeforeDelete = mocks.listOrganizations.mock.calls.length;
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));

    expect(
      await screen.findByText(
        "A stage task still has an active executor or unresolved tool outcome. Stop or recover it before deleting."
      )
    ).toBeVisible();
    expect(mocks.listOrganizations).toHaveBeenCalledTimes(readsBeforeDelete);
    expect(mocks.onReloadTargets).not.toHaveBeenCalled();
  });
});
