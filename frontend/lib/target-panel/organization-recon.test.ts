import { describe, expect, it } from "vitest";
import type { OrganizationReconRunSnapshot } from "@/lib/api/organization-recon";
import {
  applyOrganizationReconEvent,
  canExportCurrentReconAssets,
  findReconAssetsWorkbook,
  isOrganizationReconRunning,
  suggestedReconAssetsFilename,
} from "./organization-recon";

function run(
  runId: string,
  status: OrganizationReconRunSnapshot["status"]
): OrganizationReconRunSnapshot {
  return {
    runId,
    organizationId: "org-1",
    projectPath: "/tmp/project",
    status,
    stages: [],
    tasks: [],
    errors: [],
    createdAt: 1,
    updatedAt: 1,
  } satisfies OrganizationReconRunSnapshot;
}

describe("organization recon event isolation", () => {
  it("ignores a delayed event from an older run", () => {
    const current = run("new-run", "running");
    const delayed = run("old-run", "completed");

    expect(applyOrganizationReconEvent(current, delayed)).toBe(current);
  });

  it("accepts updates for the active run", () => {
    const current = run("same-run", "queued");
    const incoming = run("same-run", "running");

    expect(applyOrganizationReconEvent(current, incoming)).toBe(incoming);
    expect(isOrganizationReconRunning(incoming)).toBe(true);
  });

  it("keeps the authorized organization name in the suggested workbook filename", () => {
    expect(suggestedReconAssetsFilename("中国平安", "12345678-aaaa")).toBe(
      "中国平安-recon-assets-12345678.xlsx"
    );
  });

  it("allows current asset export after asset-intel completes without a staged run", () => {
    expect(canExportCurrentReconAssets("completed")).toBe(true);
    expect(canExportCurrentReconAssets("partial")).toBe(true);
    expect(canExportCurrentReconAssets("failed")).toBe(false);
    expect(canExportCurrentReconAssets()).toBe(false);
  });
});

describe("organization recon asset export", () => {
  it("finds the Stage 4 workbook after processing completes", () => {
    const snapshot = run("run-with-xlsx", "completed");
    snapshot.tasks.push({
      taskId: "processing",
      stage: "processing",
      sourceId: "processing",
      status: "completed",
      recordCount: 3,
      artifacts: [
        {
          path: "/tmp/.golish/tool-output/recon/run/processing/processing/exports/recon-assets.xlsx",
          kind: "asset_workbook",
          bytes: 1024n,
        },
      ],
      errors: [],
    });

    expect(findReconAssetsWorkbook(snapshot)?.bytes).toBe(1024n);
  });

  it("does not expose export before Stage 4 finishes", () => {
    const snapshot = run("run-still-processing", "running");
    snapshot.tasks.push({
      taskId: "processing",
      stage: "processing",
      sourceId: "processing",
      status: "running",
      recordCount: 0,
      artifacts: [
        {
          path: "/tmp/recon-assets.xlsx",
          kind: "asset_workbook",
          bytes: 1n,
        },
      ],
      errors: [],
    });

    expect(findReconAssetsWorkbook(snapshot)).toBeUndefined();
  });
});
